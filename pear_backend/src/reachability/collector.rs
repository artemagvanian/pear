//! Reachability
//! ====================
//!
//! This code and the documentation were adapted from the original reachability algorithm run by the
//! Rust compiler.
//!
//! This module is responsible for discovering all items that are reachable from a given root. The
//! important part here is that it not only needs to find syntax-level items (functions, structs,
//! etc) but also all their monomorphized instantiations.
//!
//! The following kinds of "mono items" are handled here:
//!
//! - Functions
//! - Methods
//! - Closures
//! - Statics
//! - Drop glue
//!
//! General Algorithm
//! -----------------
//! Let's define some terms first:
//!
//! - A "mono item" is something that results in a function or global in the LLVM IR . Mono items do
//!   not stand on their own, they can use other mono items. For example, if function `foo()` calls
//!   function `bar()` then the mono item for `foo()` uses the mono item for function `bar()`. In
//!   general, the definition for mono item A using a mono item B is that the LLVM artifact produced
//!   for A uses the LLVM artifact produced for B.
//!
//! - Mono items and the uses between them form a directed graph, where the mono items are the nodes
//!   and uses form the edges. Let's call this graph the "mono item graph".
//!
//! - The mono item graph for a program contains all mono items that are needed in order to produce
//!   the complete LLVM IR of the program.
//!
//! The purpose of the algorithm implemented in this module is to build the mono item graph from a
//! given root. Starting from the root, it finds uses by inspecting the MIR representation of the
//! item corresponding to a given node, until no more new nodes are found.
//!
//! ### Finding uses
//! Given a mono item node, we can discover uses by inspecting its MIR. We walk the MIR to find
//! other mono items used by each mono item. If the mono item we are currently at is monomorphic, we
//! also know the concrete type arguments of its used mono items. The specific forms a use can take
//! in MIR are quite diverse. Here is an overview:
//!
//! #### Calling Functions/Methods
//! The most obvious way for one mono item to use another is a function or method call (represented
//! by a CALL terminator in MIR). But calls are not the only thing that might introduce a use
//! between two function mono items, and as we will see below, they are just a specialization of the
//! form described next, and consequently will not get any special treatment in the algorithm.
//!
//! #### Taking a reference to a function or method
//! A function does not need to actually be called in order to be used by another function. It
//! suffices to just take a reference in order to introduce an edge. Consider the following example:
//!
//! ```
//! # use core::fmt::Display;
//! fn print_val<T: Display>(x: T) {
//!     println!("{}", x);
//! }
//!
//! fn call_fn(f: &dyn Fn(i32), x: i32) {
//!     f(x);
//! }
//!
//! fn main() {
//!     let print_i32 = print_val::<i32>;
//!     call_fn(&print_i32, 0);
//! }
//! ```
//! The MIR of none of these functions will contain an explicit call to `print_val::<i32>`.
//! Nonetheless, in order to mono this program, we need an instance of this function. Thus, whenever
//! we encounter a function or method in operand position, we treat it as a use of the current mono
//! item. Calls are just a special case of that.
//!
//! #### Drop glue
//! Drop glue mono items are introduced by MIR drop-statements. The generated mono item will have
//! additional drop-glue item uses if the type to be dropped contains nested values that also need
//! to be dropped. It might also have a function item use for the explicit `Drop::drop`
//! implementation of its type.
//!
//! #### Unsizing Casts
//! A subtle way of introducing use edges is by casting to a trait object. Since the resulting
//! fat-pointer contains a reference to a vtable, we need to instantiate all object-safe methods of
//! the trait, as we need to store pointers to these functions even if they never get called
//! anywhere. This can be seen as a special case of taking a function reference.

use log::trace;
use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_hir::def_id::DefId;
use rustc_hir::lang_items::LangItem;
use rustc_hir::{self as hir, Unsafety};
use rustc_middle::mir::interpret::{AllocId, ErrorHandled, GlobalAlloc, Scalar};
use rustc_middle::mir::mono::MonoItem;
use rustc_middle::mir::visit::TyContext;
use rustc_middle::mir::visit::Visitor as MirVisitor;
use rustc_middle::mir::{self, Location};
use rustc_middle::query::TyCtxtAt;
use rustc_middle::traits;
use rustc_middle::ty::adjustment::{CustomCoerceUnsized, PointerCoercion};
use rustc_middle::ty::layout::ValidityRequirement;
use rustc_middle::ty::normalize_erasing_regions::NormalizationError;
use rustc_middle::ty::{
    self, Instance, InstanceDef, Ty, TyCtxt, TypeFoldable, TypeVisitableExt, VtblEntry,
};
use rustc_middle::ty::{FnSig, GenericArgs};
use serde::Serialize;

use crate::serialize::{serialize_def_id, serialize_edges, serialize_mono_item, serialize_sig};
use crate::utils::{erase_regions_in_sig, fn_trait_method_sig};

/// We collect the specifics of how each mono item is used to aid with refinement later.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize)]
pub enum Usage<'tcx> {
    /// Root of the analysis.
    Root,
    /// Direct call via a `Call` terminator.
    Call,
    /// Drop of the item collected from a `Drop` terminator or drop of a static.
    Drop,
    /// Assert implementation collected from an `Assert` terminator.
    Assert,
    /// Unwind implementation collected from a terminator.
    Unwind,
    /// Item referenced in a block of inline assembly.
    InlineAsm,
    /// Static or a thread-local item referenced somewhere in the MIR.
    Static,
    /// Drop of the item collected from a drop item in a vtable.
    IndirectDrop,
    /// Thread-local shim generated by the compiler for some thread local.
    ThreadLocalShim,
    /// Static function collected from a compile time function evaluation alloc.
    StaticFn {
        #[serde(serialize_with = "serialize_sig")]
        sig: FnSig<'tcx>,
    },
    /// Function (or closure) pointer produced by taking a reference to a function (or closure).
    FnPtr {
        #[serde(serialize_with = "serialize_sig")]
        sig: FnSig<'tcx>,
    },
    /// Vtable item produced by an unsize cast.
    VtableItem {
        #[serde(serialize_with = "serialize_def_id")]
        trait_def_id: DefId,
        impl_type: ImplType,
    },
    FnTraitItem {
        #[serde(serialize_with = "serialize_sig")]
        sig: FnSig<'tcx>,
    },
    StaticClosureShim {
        #[serde(serialize_with = "serialize_sig")]
        sig: FnSig<'tcx>,
    },
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize)]

/// Differentiates between methods coming from an impl block and inherent ones.
pub enum ImplType {
    Explicit {
        #[serde(serialize_with = "serialize_def_id")]
        def_id: DefId,
    },
    Inherent,
}

/// Mono item with usage specifics attached.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize)]
pub struct Node<'tcx> {
    #[serde(serialize_with = "serialize_mono_item")]
    item: MonoItem<'tcx>,
    usage: Usage<'tcx>,
}

impl<'tcx> Node<'tcx> {
    pub fn new(item: MonoItem<'tcx>, usage: Usage<'tcx>) -> Self {
        Self { item, usage }
    }

    pub fn item(&self) -> MonoItem<'tcx> {
        self.item
    }

    pub fn usage(&self) -> Usage<'tcx> {
        self.usage
    }

    /// Returns true if the mono item was not collected as a result of a direct invocation via a
    /// terminator.
    pub fn is_indirect(&self) -> bool {
        matches!(
            self.usage,
            Usage::StaticFn { .. }
                | Usage::VtableItem { .. }
                | Usage::FnTraitItem { .. }
                | Usage::FnPtr { .. }
                | Usage::StaticClosureShim { .. }
        )
    }

    /// Resolves a callable instance from a mono item if one exists.
    pub fn expect_instance(&self) -> Instance<'tcx> {
        match self.item {
            MonoItem::Fn(instance) => instance,
            MonoItem::Static(..) | MonoItem::GlobalAsm(..) => bug!(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UsageGraph<'tcx> {
    // Maps every mono item to the mono items used by it.
    #[serde(serialize_with = "serialize_edges")]
    forward_edges: FxHashMap<MonoItem<'tcx>, FxHashSet<Node<'tcx>>>,

    // Maps every mono item to the mono items that use it.
    #[serde(serialize_with = "serialize_edges")]
    backward_edges: FxHashMap<MonoItem<'tcx>, FxHashSet<Node<'tcx>>>,
}

type UsedMonoItems<'tcx> = Vec<Node<'tcx>>;

impl<'tcx> UsageGraph<'tcx> {
    fn new() -> UsageGraph<'tcx> {
        UsageGraph {
            forward_edges: FxHashMap::default(),
            backward_edges: FxHashMap::default(),
        }
    }

    fn record_used<'a>(&mut self, user_item: Node<'tcx>, used_items: Vec<Node<'tcx>>)
    where
        'tcx: 'a,
    {
        for used_item in used_items.iter() {
            self.backward_edges
                .entry(used_item.item())
                .or_default()
                .insert(user_item);
        }

        self.forward_edges
            .entry(user_item.item())
            .or_default()
            .extend(used_items.into_iter());
    }
}

/// Collect all monomorphized items reachable from `starting_item`.
fn collect_items_rec<'tcx>(
    tcx: TyCtxt<'tcx>,
    starting_item: Node<'tcx>,
    visited: &mut FxHashSet<Node<'tcx>>,
    usage_map: &mut UsageGraph<'tcx>,
) {
    if !visited.insert(starting_item) {
        // We've been here already, no need to search again.
        return;
    }

    if tcx.is_foreign_item(starting_item.item().def_id()) {
        // A foreign item has no body.
        return;
    }

    let mut used_items = Vec::new();

    match starting_item.item() {
        MonoItem::Fn(instance) => {
            rustc_data_structures::stack::ensure_sufficient_stack(|| {
                collect_used_items(tcx, instance, starting_item.usage(), &mut used_items);
            });
        }
        MonoItem::Static(def_id) => {
            let instance = Instance::mono(tcx, def_id);
            let ty = instance.ty(tcx, ty::ParamEnv::reveal_all());
            visit_drop_use(tcx, ty, true, &mut used_items, Usage::Drop);

            if let Ok(alloc) = tcx.eval_static_initializer(def_id) {
                for &prov in alloc.inner().provenance().ptrs().values() {
                    collect_alloc(tcx, prov.alloc_id(), &mut used_items);
                }
            }

            if tcx.needs_thread_local_shim(def_id) {
                used_items.push(Node::new(
                    MonoItem::Fn(Instance {
                        def: InstanceDef::ThreadLocalShim(def_id),
                        args: GenericArgs::empty(),
                    }),
                    Usage::ThreadLocalShim,
                ));
            }
        }
        MonoItem::GlobalAsm(item_id) => {
            let item = tcx.hir().item(item_id);
            if let hir::ItemKind::GlobalAsm(asm) = item.kind {
                for (op, op_sp) in asm.operands {
                    match op {
                        hir::InlineAsmOperand::Const { .. } => {
                            // Only constants which resolve to a plain integer
                            // are supported. Therefore the value should not
                            // depend on any other items.
                        }
                        hir::InlineAsmOperand::SymFn { anon_const } => {
                            let fn_ty = tcx
                                .typeck_body(anon_const.body)
                                .node_type(anon_const.hir_id);
                            visit_fn_use(tcx, fn_ty, false, &mut used_items, Usage::InlineAsm);
                        }
                        hir::InlineAsmOperand::SymStatic { path: _, def_id } => {
                            trace!("collecting static {:?}", def_id);
                            used_items.push(Node::new(MonoItem::Static(*def_id), Usage::InlineAsm));
                        }
                        hir::InlineAsmOperand::In { .. }
                        | hir::InlineAsmOperand::Out { .. }
                        | hir::InlineAsmOperand::InOut { .. }
                        | hir::InlineAsmOperand::SplitInOut { .. } => {
                            span_bug!(*op_sp, "invalid operand type for global_asm!")
                        }
                    }
                }
            } else {
                span_bug!(
                    item.span,
                    "Mismatch between hir::Item type and MonoItem type"
                )
            }
        }
    }

    usage_map.record_used(starting_item, used_items.clone());

    for used_item in used_items {
        collect_items_rec(tcx, used_item, visited, usage_map);
    }
}

struct MirUsedCollector<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a mir::Body<'tcx>,
    output: &'a mut UsedMonoItems<'tcx>,
    instance: Instance<'tcx>,
    usage: Usage<'tcx>,
}

impl<'a, 'tcx> MirUsedCollector<'a, 'tcx> {
    pub fn monomorphize<T>(&self, value: T) -> Result<T, NormalizationError<'tcx>>
    where
        T: TypeFoldable<TyCtxt<'tcx>>,
    {
        trace!("monomorphize: self.instance={:?}", self.instance);
        let maybe_mono = self
            .instance
            .try_instantiate_mir_and_normalize_erasing_regions(
                self.tcx,
                ty::ParamEnv::reveal_all(),
                ty::EarlyBinder::bind(value),
            );
        Ok(maybe_mono.expect("reachability is not configured to perform partial resolution"))
    }
}

impl<'a, 'tcx> MirVisitor<'tcx> for MirUsedCollector<'a, 'tcx> {
    fn visit_rvalue(&mut self, rvalue: &mir::Rvalue<'tcx>, location: Location) {
        trace!("visiting rvalue {:?}", *rvalue);

        let span = self.body.source_info(location).span;

        match *rvalue {
            // When doing an cast from a regular pointer to a fat pointer, we
            // have to instantiate all methods of the trait being cast to, so we
            // can build the appropriate vtable.
            mir::Rvalue::Cast(
                mir::CastKind::PointerCoercion(PointerCoercion::Unsize),
                ref operand,
                target_ty,
            )
            | mir::Rvalue::Cast(mir::CastKind::DynStar, ref operand, target_ty) => {
                let Ok(target_ty) = self.monomorphize(target_ty) else {
                    return;
                };
                let source_ty = operand.ty(self.body, self.tcx);
                let Ok(source_ty) = self.monomorphize(source_ty) else {
                    return;
                };
                let (source_ty, target_ty) =
                    find_vtable_types_for_unsizing(self.tcx.at(span), source_ty, target_ty);
                // This could also be a different Unsize instruction, like
                // from a fixed sized array to a slice. But we are only
                // interested in things that produce a vtable.
                if (target_ty.is_trait() && !source_ty.is_trait())
                    || (target_ty.is_dyn_star() && !source_ty.is_dyn_star())
                {
                    create_mono_items_for_vtable_methods(
                        self.tcx,
                        target_ty,
                        source_ty,
                        self.output,
                    );
                }
            }
            mir::Rvalue::Cast(
                mir::CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer),
                ref operand,
                _,
            ) => {
                let fn_ty = operand.ty(self.body, self.tcx);
                let Ok(fn_ty) = self.monomorphize(fn_ty) else {
                    return;
                };
                let sig = match fn_ty.kind() {
                    ty::FnDef(def_id, args) => erase_regions_in_sig(
                        self.tcx.fn_sig(def_id).instantiate(self.tcx, args),
                        self.tcx,
                    ),
                    _ => bug!(),
                };
                visit_fn_use(self.tcx, fn_ty, false, self.output, Usage::FnPtr { sig });
            }
            mir::Rvalue::Cast(
                mir::CastKind::PointerCoercion(PointerCoercion::ClosureFnPointer(_)),
                ref operand,
                _,
            ) => {
                let source_ty = operand.ty(self.body, self.tcx);
                let Ok(source_ty) = self.monomorphize(source_ty) else {
                    return;
                };
                match *source_ty.kind() {
                    ty::Closure(def_id, args) => {
                        let instance = Instance::resolve_closure(
                            self.tcx,
                            def_id,
                            args,
                            ty::ClosureKind::FnOnce,
                        )
                        .expect("failed to normalize and resolve closure during codegen");
                        let sig = erase_regions_in_sig(
                            self.tcx
                                .signature_unclosure(args.as_closure().sig(), Unsafety::Normal),
                            self.tcx,
                        );
                        self.output
                            .push(create_fn_mono_item(instance, Usage::FnPtr { sig }));
                    }
                    _ => bug!(),
                }
            }
            mir::Rvalue::ThreadLocalRef(def_id) => {
                assert!(self.tcx.is_thread_local_static(def_id));
                trace!("collecting thread-local static {:?}", def_id);
                self.output
                    .push(Node::new(MonoItem::Static(def_id), Usage::Static));
            }
            _ => { /* not interesting */ }
        }

        self.super_rvalue(rvalue, location);
    }

    /// This does not walk the constant, as it has been handled entirely here and trying
    /// to walk it would attempt to evaluate the `ty::Const` inside, which doesn't necessarily
    /// work, as some constants cannot be represented in the type system.
    fn visit_constant(&mut self, constant: &mir::ConstOperand<'tcx>, location: Location) {
        let Ok(const_) = self.monomorphize(constant.const_) else {
            return;
        };
        let param_env = ty::ParamEnv::reveal_all();
        let val = match const_.eval(self.tcx, param_env, None) {
            Ok(v) => v,
            Err(ErrorHandled::Reported(..)) => return,
            Err(ErrorHandled::TooGeneric(..)) => span_bug!(
                self.body.source_info(location).span,
                "collection encountered polymorphic constant: {:?}",
                const_
            ),
        };
        collect_const_value(self.tcx, val, self.output);
        MirVisitor::visit_ty(self, const_.ty(), TyContext::Location(location));
    }

    fn visit_terminator(&mut self, terminator: &mir::Terminator<'tcx>, location: Location) {
        trace!("visiting terminator {:?} @ {:?}", terminator, location);
        let tcx = self.tcx;
        let push_mono_lang_item = |this: &mut Self, lang_item: LangItem, usage: Usage<'tcx>| {
            let instance = Instance::mono(tcx, tcx.require_lang_item(lang_item, None));
            this.output.push(create_fn_mono_item(instance, usage));
        };

        match terminator.kind {
            mir::TerminatorKind::Call { ref func, .. } => {
                let callee_ty = func.ty(self.body, tcx);
                let Ok(callee_ty) = self.monomorphize(callee_ty) else {
                    return;
                };
                let is_static_closure_shim = matches!(self.usage, Usage::StaticFn { .. })
                    && matches!(self.instance.def, InstanceDef::ClosureOnceShim { .. });
                let usage = if is_static_closure_shim {
                    let sig = match self.instance.args[0].as_type().unwrap().kind() {
                        ty::Closure(_, args) => erase_regions_in_sig(
                            self.tcx
                                .signature_unclosure(args.as_closure().sig(), Unsafety::Normal),
                            self.tcx,
                        ),
                        _ => bug!(),
                    };
                    Usage::StaticClosureShim { sig }
                } else {
                    Usage::Call
                };
                visit_fn_use(self.tcx, callee_ty, true, self.output, usage)
            }
            mir::TerminatorKind::Drop { ref place, .. } => {
                let ty = place.ty(self.body, self.tcx).ty;
                let Ok(ty) = self.monomorphize(ty) else {
                    return;
                };
                visit_drop_use(self.tcx, ty, true, self.output, Usage::Drop);
            }
            mir::TerminatorKind::InlineAsm { ref operands, .. } => {
                for op in operands {
                    match *op {
                        mir::InlineAsmOperand::SymFn { ref value } => {
                            let Ok(fn_ty) = self.monomorphize(value.const_.ty()) else {
                                return;
                            };
                            visit_fn_use(self.tcx, fn_ty, false, self.output, Usage::InlineAsm);
                        }
                        mir::InlineAsmOperand::SymStatic { def_id } => {
                            trace!("collecting asm sym static {:?}", def_id);
                            self.output
                                .push(Node::new(MonoItem::Static(def_id), Usage::InlineAsm));
                        }
                        _ => {}
                    }
                }
            }
            mir::TerminatorKind::Assert { ref msg, .. } => {
                let lang_item = match &**msg {
                    mir::AssertKind::BoundsCheck { .. } => LangItem::PanicBoundsCheck,
                    mir::AssertKind::MisalignedPointerDereference { .. } => {
                        LangItem::PanicMisalignedPointerDereference
                    }
                    _ => LangItem::Panic,
                };
                push_mono_lang_item(self, lang_item, Usage::Assert);
            }
            mir::TerminatorKind::UnwindTerminate(reason) => {
                push_mono_lang_item(self, reason.lang_item(), Usage::Unwind);
            }
            mir::TerminatorKind::Goto { .. }
            | mir::TerminatorKind::SwitchInt { .. }
            | mir::TerminatorKind::UnwindResume
            | mir::TerminatorKind::Return
            | mir::TerminatorKind::Unreachable => {}
            mir::TerminatorKind::CoroutineDrop
            | mir::TerminatorKind::Yield { .. }
            | mir::TerminatorKind::FalseEdge { .. }
            | mir::TerminatorKind::FalseUnwind { .. } => bug!(),
        }

        if let Some(mir::UnwindAction::Terminate(reason)) = terminator.unwind() {
            push_mono_lang_item(self, reason.lang_item(), Usage::Unwind);
        }

        self.super_terminator(terminator, location);
    }
}

fn visit_drop_use<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    is_direct_call: bool,
    output: &mut UsedMonoItems<'tcx>,
    usage: Usage<'tcx>,
) {
    let def_id = tcx.require_lang_item(LangItem::DropInPlace, None);
    let args = tcx.mk_args(&[ty.into()]);
    let instance = if let Ok(Some(instance)) =
        ty::Instance::resolve(tcx, ty::ParamEnv::reveal_all(), def_id, args)
    {
        instance
    } else {
        bug!("reachability is not configured to perform partial resolution")
    };

    visit_instance_use(tcx, instance, is_direct_call, output, usage);
}

fn visit_fn_use<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    is_direct_call: bool,
    output: &mut UsedMonoItems<'tcx>,
    usage: Usage<'tcx>,
) {
    if let ty::FnDef(def_id, args) = *ty.kind() {
        let instance = if is_direct_call {
            if let Ok(Some(instance)) =
                ty::Instance::resolve(tcx, ty::ParamEnv::reveal_all(), def_id, args)
            {
                instance
            } else {
                bug!("reachability is not configured to perform partial resolution")
            }
        } else {
            match ty::Instance::resolve_for_fn_ptr(tcx, ty::ParamEnv::reveal_all(), def_id, args) {
                Some(instance) => instance,
                _ => bug!("failed to resolve instance for {ty}"),
            }
        };
        visit_instance_use(tcx, instance, is_direct_call, output, usage);
    }
}

fn visit_instance_use<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: ty::Instance<'tcx>,
    is_direct_call: bool,
    output: &mut UsedMonoItems<'tcx>,
    usage: Usage<'tcx>,
) {
    trace!(
        "visit_item_use({:?}, is_direct_call={:?})",
        instance,
        is_direct_call
    );

    // The intrinsics assert_inhabited, assert_zero_valid, and assert_mem_uninitialized_valid will
    // be lowered in codegen to nothing or a call to panic_nounwind. So if we encounter any
    // of those intrinsics, we need to include a mono item for panic_nounwind, else we may try to
    // codegen a call to that function without generating code for the function itself.
    if let ty::InstanceDef::Intrinsic(def_id) = instance.def {
        let name = tcx.item_name(def_id);
        if let Some(_requirement) = ValidityRequirement::from_intrinsic(name) {
            let def_id = tcx.lang_items().get(LangItem::PanicNounwind).unwrap();
            let panic_instance = Instance::mono(tcx, def_id);
            output.push(create_fn_mono_item(panic_instance, usage));
        }
    }

    match instance.def {
        ty::InstanceDef::Virtual(..) | ty::InstanceDef::Intrinsic(_) => {
            if !is_direct_call {
                bug!("{:?} being reified", instance);
            }
        }
        ty::InstanceDef::ThreadLocalShim(..) => {
            bug!("{:?} being reified", instance);
        }
        ty::InstanceDef::DropGlue(_, None) => {
            // Don't need to emit noop drop glue if we are calling directly.
            if !is_direct_call {
                output.push(create_fn_mono_item(instance, usage));
            }
        }
        ty::InstanceDef::DropGlue(_, Some(_))
        | ty::InstanceDef::VTableShim(..)
        | ty::InstanceDef::ReifyShim(..)
        | ty::InstanceDef::ClosureOnceShim { .. }
        | ty::InstanceDef::Item(..)
        | ty::InstanceDef::FnPtrShim(..)
        | ty::InstanceDef::CloneShim(..)
        | ty::InstanceDef::FnPtrAddrShim(..) => {
            output.push(create_fn_mono_item(instance, usage));
        }
    }
}

/// For a given pair of source and target type that occur in an unsizing coercion,
/// this function finds the pair of types that determines the vtable linking
/// them.
///
/// For example, the source type might be `&SomeStruct` and the target type
/// might be `&dyn SomeTrait` in a cast like:
///
/// ```rust,ignore (not real code)
/// let src: &SomeStruct = ...;
/// let target = src as &dyn SomeTrait;
/// ```
///
/// Then the output of this function would be (SomeStruct, SomeTrait) since for
/// constructing the `target` fat-pointer we need the vtable for that pair.
///
/// Things can get more complicated though because there's also the case where
/// the unsized type occurs as a field:
///
/// ```rust
/// struct ComplexStruct<T: ?Sized> {
///    a: u32,
///    b: f64,
///    c: T
/// }
/// ```
///
/// In this case, if `T` is sized, `&ComplexStruct<T>` is a thin pointer. If `T`
/// is unsized, `&SomeStruct` is a fat pointer, and the vtable it points to is
/// for the pair of `T` (which is a trait) and the concrete type that `T` was
/// originally coerced from:
///
/// ```rust,ignore (not real code)
/// let src: &ComplexStruct<SomeStruct> = ...;
/// let target = src as &ComplexStruct<dyn SomeTrait>;
/// ```
///
/// Again, we want this `find_vtable_types_for_unsizing()` to provide the pair
/// `(SomeStruct, SomeTrait)`.
///
/// Finally, there is also the case of custom unsizing coercions, e.g., for
/// smart pointers such as `Rc` and `Arc`.
fn find_vtable_types_for_unsizing<'tcx>(
    tcx: TyCtxtAt<'tcx>,
    source_ty: Ty<'tcx>,
    target_ty: Ty<'tcx>,
) -> (Ty<'tcx>, Ty<'tcx>) {
    let ptr_vtable = |inner_source: Ty<'tcx>, inner_target: Ty<'tcx>| {
        let param_env = ty::ParamEnv::reveal_all();
        let type_has_metadata = |ty: Ty<'tcx>| -> bool {
            if ty.is_sized(tcx.tcx, param_env) {
                return false;
            }
            let tail = tcx.struct_tail_erasing_lifetimes(ty, param_env);
            match tail.kind() {
                ty::Foreign(..) => false,
                ty::Str | ty::Slice(..) | ty::Dynamic(..) => true,
                _ => bug!("unexpected unsized tail: {:?}", tail),
            }
        };
        if type_has_metadata(inner_source) {
            (inner_source, inner_target)
        } else {
            tcx.struct_lockstep_tails_erasing_lifetimes(inner_source, inner_target, param_env)
        }
    };

    match (&source_ty.kind(), &target_ty.kind()) {
        (&ty::Ref(_, a, _), &ty::Ref(_, b, _) | &ty::RawPtr(ty::TypeAndMut { ty: b, .. }))
        | (&ty::RawPtr(ty::TypeAndMut { ty: a, .. }), &ty::RawPtr(ty::TypeAndMut { ty: b, .. })) => {
            ptr_vtable(*a, *b)
        }
        (&ty::Adt(def_a, _), &ty::Adt(def_b, _)) if def_a.is_box() && def_b.is_box() => {
            ptr_vtable(source_ty.boxed_ty(), target_ty.boxed_ty())
        }

        // T as dyn* Trait
        (_, &ty::Dynamic(_, _, ty::DynStar)) => ptr_vtable(source_ty, target_ty),

        (&ty::Adt(source_adt_def, source_args), &ty::Adt(target_adt_def, target_args)) => {
            assert_eq!(source_adt_def, target_adt_def);

            let CustomCoerceUnsized::Struct(coerce_index) =
                custom_coerce_unsize_info(tcx, source_ty, target_ty);

            let source_fields = &source_adt_def.non_enum_variant().fields;
            let target_fields = &target_adt_def.non_enum_variant().fields;

            assert!(
                coerce_index.index() < source_fields.len()
                    && source_fields.len() == target_fields.len()
            );

            find_vtable_types_for_unsizing(
                tcx,
                source_fields[coerce_index].ty(*tcx, source_args),
                target_fields[coerce_index].ty(*tcx, target_args),
            )
        }
        _ => bug!(
            "find_vtable_types_for_unsizing: invalid coercion {:?} -> {:?}",
            source_ty,
            target_ty
        ),
    }
}

fn create_fn_mono_item<'tcx>(instance: Instance<'tcx>, usage: Usage<'tcx>) -> Node<'tcx> {
    Node::new(MonoItem::Fn(instance), usage)
}

/// Creates a `MonoItem` for each method that is referenced by the vtable for
/// the given trait/impl pair.
fn create_mono_items_for_vtable_methods<'tcx>(
    tcx: TyCtxt<'tcx>,
    trait_ty: Ty<'tcx>,
    impl_ty: Ty<'tcx>,
    output: &mut UsedMonoItems<'tcx>,
) {
    assert!(!trait_ty.has_escaping_bound_vars() && !impl_ty.has_escaping_bound_vars());

    if let ty::Dynamic(trait_ty, ..) = trait_ty.kind() {
        if let Some(principal) = trait_ty.principal() {
            let poly_trait_ref = principal.with_self_ty(tcx, impl_ty);
            assert!(!poly_trait_ref.has_escaping_bound_vars());

            // Walk all methods of the trait, including those of its supertraits
            let entries = tcx.vtable_entries(poly_trait_ref);
            let methods = entries
                .iter()
                .filter_map(|entry| match entry {
                    VtblEntry::MetadataDropInPlace
                    | VtblEntry::MetadataSize
                    | VtblEntry::MetadataAlign
                    | VtblEntry::Vacant => None,
                    VtblEntry::TraitVPtr(_) => {
                        // all super trait items already covered, so skip them.
                        None
                    }
                    VtblEntry::Method(instance) => Some(*instance),
                })
                .map(|item| {
                    let usage = {
                        // Record def_id of the trait where the method is coming from.
                        let trait_def_id = tcx
                            .impl_of_method(item.def_id())
                            .and_then(|impl_id| tcx.trait_id_of_impl(impl_id))
                            .unwrap_or(poly_trait_ref.def_id());
                        if tcx.is_fn_trait(trait_def_id) {
                            // Need to record function signature of the Fn-like trait implementor.
                            Usage::FnTraitItem {
                                sig: fn_trait_method_sig(item.def_id(), item.args, tcx),
                            }
                        } else {
                            // Record def_id of the impl block where the method is coming from.
                            let impl_type = tcx
                                .impl_of_method(item.def_id())
                                .map(|impl_id| ImplType::Explicit { def_id: impl_id })
                                .unwrap_or(ImplType::Inherent);
                            Usage::VtableItem {
                                trait_def_id,
                                impl_type,
                            }
                        }
                    };
                    create_fn_mono_item(item, usage)
                });
            output.extend(methods);
        }

        // Also add the destructor.
        visit_drop_use(tcx, impl_ty, false, output, Usage::IndirectDrop);
    }
}

/// Scans the CTFE alloc in order to find function calls, closures, and drop-glue.
fn collect_alloc<'tcx>(tcx: TyCtxt<'tcx>, alloc_id: AllocId, output: &mut UsedMonoItems<'tcx>) {
    match tcx.global_alloc(alloc_id) {
        GlobalAlloc::Static(def_id) => {
            assert!(!tcx.is_thread_local_static(def_id));
            trace!("collecting static {:?}", def_id);
            output.push(Node::new(MonoItem::Static(def_id), Usage::Static));
        }
        GlobalAlloc::Memory(alloc) => {
            trace!("collecting {:?} with {:#?}", alloc_id, alloc);
            for &prov in alloc.inner().provenance().ptrs().values() {
                rustc_data_structures::stack::ensure_sufficient_stack(|| {
                    collect_alloc(tcx, prov.alloc_id(), output);
                });
            }
        }
        GlobalAlloc::Function(fn_instance) => {
            trace!("collecting {:?} with {:#?}", alloc_id, fn_instance);
            let sig = erase_regions_in_sig(
                tcx.fn_sig(fn_instance.def_id())
                    .instantiate(tcx, fn_instance.args),
                tcx,
            );
            output.push(create_fn_mono_item(fn_instance, Usage::StaticFn { sig }));
        }
        GlobalAlloc::VTable(ty, trait_ref) => {
            let alloc_id = tcx.vtable_allocation((ty, trait_ref));
            collect_alloc(tcx, alloc_id, output)
        }
    }
}

/// Scans the MIR in order to find function calls, closures, and drop-glue.
fn collect_used_items<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    usage: Usage<'tcx>,
    output: &mut UsedMonoItems<'tcx>,
) {
    let body = tcx.instance_mir(instance.def);
    // Here we rely on the visitor also visiting `required_consts`, so that we evaluate them
    // and abort compilation if any of them errors.
    MirUsedCollector {
        tcx,
        body,
        output,
        instance,
        usage,
    }
    .visit_body(body);
}

fn collect_const_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    value: mir::ConstValue<'tcx>,
    output: &mut UsedMonoItems<'tcx>,
) {
    match value {
        mir::ConstValue::Scalar(Scalar::Ptr(ptr, _size)) => {
            collect_alloc(tcx, ptr.provenance.alloc_id(), output)
        }
        mir::ConstValue::Indirect { alloc_id, .. } => collect_alloc(tcx, alloc_id, output),
        mir::ConstValue::Slice { data, meta: _ } => {
            for &prov in data.inner().provenance().ptrs().values() {
                collect_alloc(tcx, prov.alloc_id(), output);
            }
        }
        _ => {}
    }
}

pub fn collect_from<'tcx>(
    tcx: TyCtxt<'tcx>,
    root: MonoItem<'tcx>,
) -> (FxHashSet<Node<'tcx>>, UsageGraph<'tcx>) {
    let mut visited = FxHashSet::default();
    let mut usage_map = UsageGraph::new();
    collect_items_rec(
        tcx,
        Node::new(root, Usage::Root),
        &mut visited,
        &mut usage_map,
    );
    (visited, usage_map)
}

fn custom_coerce_unsize_info<'tcx>(
    tcx: TyCtxtAt<'tcx>,
    source_ty: Ty<'tcx>,
    target_ty: Ty<'tcx>,
) -> CustomCoerceUnsized {
    let trait_ref = ty::TraitRef::from_lang_item(
        tcx.tcx,
        LangItem::CoerceUnsized,
        tcx.span,
        [source_ty, target_ty],
    );

    match tcx.codegen_select_candidate((ty::ParamEnv::reveal_all(), trait_ref)) {
        Ok(traits::ImplSource::UserDefined(traits::ImplSourceUserDefinedData {
            impl_def_id,
            ..
        })) => tcx.coerce_unsized_info(impl_def_id).custom_kind.unwrap(),
        impl_source => {
            panic!("invalid `CoerceUnsized` impl_source: {:?}", impl_source);
        }
    }
}
