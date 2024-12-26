use std::fs;

use rustc_hash::{FxHashMap, FxHashSet};
use rustc_hir::def_id::DefId;
use rustc_middle::{
    mir::{visit::Visitor, Body, Location, Operand, Terminator, TerminatorKind},
    ty::{
        self, EarlyBinder, FnSig, Instance, InstanceDef, ParamEnv, Ty, TyCtxt, TyKind, TypeFoldable,
    },
};
use rustc_span::Span;
use serde::Serialize;

use crate::reachability::{Usage, UsedMonoItem};
use crate::serialize::{
    serialize_instance, serialize_instance_vec, serialize_refined_edges, serialize_span,
    serialize_ty,
};

mod utils;
use utils::*;

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize)]
pub enum RefinedNode<'tcx> {
    Concrete {
        #[serde(serialize_with = "serialize_instance")]
        instance: Instance<'tcx>,
        #[serde(serialize_with = "serialize_span")]
        span: Span,
    },
    Refined {
        #[serde(serialize_with = "serialize_instance_vec")]
        instances: Vec<Instance<'tcx>>,
        #[serde(serialize_with = "serialize_span")]
        span: Span,
    },
    Ambiguous {
        #[serde(serialize_with = "serialize_ty")]
        fn_ty: Ty<'tcx>,
        #[serde(serialize_with = "serialize_span")]
        span: Span,
    },
}

impl<'tcx> RefinedNode<'tcx> {
    pub fn into_vec(self) -> Vec<Instance<'tcx>> {
        match self {
            RefinedNode::Concrete { instance, .. } => vec![instance],
            RefinedNode::Refined { instances, .. } => instances,
            RefinedNode::Ambiguous { .. } => vec![],
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RefinedUsageGraph<'tcx> {
    // Maps every instance to the instances used by it.
    #[serde(serialize_with = "serialize_refined_edges")]
    forward_edges: FxHashMap<Instance<'tcx>, FxHashSet<RefinedNode<'tcx>>>,
}

impl<'tcx> RefinedUsageGraph<'tcx> {
    fn new() -> Self {
        Self {
            forward_edges: FxHashMap::default(),
        }
    }

    fn add_edge(&mut self, from: Instance<'tcx>, to: &RefinedNode<'tcx>) {
        self.forward_edges
            .entry(from)
            .or_default()
            .insert(to.clone());
    }
}

#[derive(Debug, Serialize)]
pub struct StackItem<'tcx> {
    #[serde(serialize_with = "serialize_instance")]
    instance: Instance<'tcx>,
    #[serde(serialize_with = "serialize_span")]
    span: Span,
}

impl<'tcx> StackItem<'tcx> {
    pub fn new(instance: Instance<'tcx>, span: Span) -> Self {
        Self { instance, span }
    }
}

pub struct RefinerVisitor<'tcx> {
    current_instance: Instance<'tcx>,
    current_body: Body<'tcx>,
    reachable_indirect: FxHashSet<UsedMonoItem<'tcx>>,
    refined_usage_graph: RefinedUsageGraph<'tcx>,
    call_stack: Vec<StackItem<'tcx>>,
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> RefinerVisitor<'tcx> {
    pub fn new(
        root: Instance<'tcx>,
        reachable: FxHashSet<UsedMonoItem<'tcx>>,
        tcx: TyCtxt<'tcx>,
    ) -> Self {
        // We do not instantiate and normalize body just yet but do it lazily instead to support
        // partially parametric instances.
        let root_body = tcx.instance_mir(root.def).clone();

        // Find all reachable mono items that were not used directly, they will be used when
        // resolving ambiguous calls.
        let reachable_indirect = reachable
            .into_iter()
            .filter(|used_mono_item| used_mono_item.is_indirect())
            .collect();

        Self {
            current_instance: root,
            current_body: root_body,
            reachable_indirect,
            refined_usage_graph: RefinedUsageGraph::new(),
            call_stack: vec![StackItem::new(root, tcx.def_span(root.def_id()))],
            tcx,
        }
    }

    pub fn refine(mut self) -> RefinedUsageGraph<'tcx> {
        self.visit_body(&self.current_body.clone());
        self.refined_usage_graph
    }

    fn candidates_for_ambiguous(
        &self,
        ambiguous_fn_sig: FnSig<'tcx>,
    ) -> Option<Vec<Instance<'tcx>>> {
        // Check whether a reachable indirect item could be used to resolve the ambiguous one.
        let refined_candidates: Vec<Instance<'tcx>> = self
            .reachable_indirect
            .iter()
            .filter_map(|reachable_indirect| {
                // Try instantiating the signature of an instance with generic args in scope.
                match reachable_indirect.usage() {
                    Usage::StaticFn {
                        sig: indirect_fn_sig,
                    }
                    | Usage::FnPtr {
                        sig: indirect_fn_sig,
                    }
                    | Usage::StaticClosureShim {
                        sig: indirect_fn_sig,
                    } => {
                        if is_resolution_of(ambiguous_fn_sig, indirect_fn_sig) {
                            Some(reachable_indirect.into_instance())
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
            .collect();

        if refined_candidates.is_empty() {
            None
        } else {
            Some(refined_candidates)
        }
    }

    fn candidates_for_virtual(
        &self,
        trait_def_id: DefId,
        vtable_pos: usize,
    ) -> Option<Vec<Instance<'tcx>>> {
        let refined_candidates: Vec<Instance<'tcx>> = self
            .reachable_indirect
            .iter()
            .filter(|reachable_indirect| match reachable_indirect.usage() {
                Usage::VtableItem {
                    trait_def_id: indirect_trait_def_id,
                    vtable_pos: indirect_vtable_pos,
                } => indirect_trait_def_id == trait_def_id && vtable_pos == indirect_vtable_pos,
                _ => false,
            })
            .map(|used_mono_item| used_mono_item.into_instance())
            .collect();

        if refined_candidates.is_empty() {
            None
        } else {
            Some(refined_candidates)
        }
    }

    fn instantiate_with_current_instance<T: TypeFoldable<TyCtxt<'tcx>>>(
        &self,
        v: EarlyBinder<T>,
    ) -> T {
        self.current_instance
            .instantiate_mir_and_normalize_erasing_regions(self.tcx, ParamEnv::reveal_all(), v)
    }

    fn refine_rec(&mut self, func: &Operand<'tcx>, _args: &Vec<Operand<'tcx>>, span: Span) {
        // Refine the passed function operand.
        let fn_ty = self.instantiate_with_current_instance(EarlyBinder::bind(
            func.ty(&self.current_body, self.tcx),
        ));

        let refined = match fn_ty.kind().clone() {
            TyKind::FnDef(def_id, generic_args) => {
                let instance = ty::Instance::expect_resolve(
                    self.tcx,
                    ParamEnv::reveal_all(),
                    def_id,
                    generic_args,
                );
                match instance.def {
                    InstanceDef::Virtual(method_def_id, vtable_pos) => {
                        let trait_def_id = self
                            .tcx
                            .trait_of_item(method_def_id)
                            .expect("InstanceDef::Virtual does not contain a trait method");
                        match self.candidates_for_virtual(trait_def_id, vtable_pos) {
                            Some(instances) => RefinedNode::Refined { instances, span },
                            None => {
                                RefinedNode::Ambiguous { fn_ty, span }
                            }
                        }
                    }
                    _ => RefinedNode::Concrete { instance, span },
                }
            }
            TyKind::FnPtr(poly_fn_sig) => {
                let fn_sig = self
                    .tcx
                    .instantiate_bound_regions_with_erased(self.tcx.erase_regions(poly_fn_sig));
                match self.candidates_for_ambiguous(fn_sig) {
                    Some(instances) => RefinedNode::Refined { instances, span },
                    None => {
                        RefinedNode::Ambiguous { fn_ty, span }
                    }
                }
            }
            _ => self.panic_and_dump_call_stack(
                "unexpected callee type encountered when performing refinement",
            ),
        };

        // Skip the function if it is already in the usage graph.
        if self
            .refined_usage_graph
            .forward_edges
            .get(&self.current_instance)
            .is_some_and(|s| s.contains(&refined))
        {
            return;
        }

        // Add the edge to the refined graph.
        self.refined_usage_graph
            .add_edge(self.current_instance, &refined);

        for callee in refined.into_vec() {
            // Resolved callee should not be virtual.
            if is_virtual(callee) {
                self.panic_and_dump_call_stack(
                    "resolved to a virtual callee when performing refinement",
                );
            }

            // Skip recurring into the item if the item does not have a body.
            if self.tcx.is_foreign_item(callee.def_id()) || is_intrinsic(callee) {
                continue;
            }

            // We do not instantiate and normalize body just yet but do it lazily instead to support
            // partially parametric instances.
            let callee_body = self.tcx.instance_mir(callee.def).clone();

            // Save previous instance and previous body to swap in later.
            let previous_instance = self.current_instance;
            let previous_body = self.current_body.clone();

            // Swap root & body for the refined instance.
            self.current_instance = callee;
            self.current_body = callee_body;

            // Add callee to the call stack.
            self.call_stack
                .push(StackItem::new(callee, self.tcx.def_span(callee.def_id())));

            // Continue collection.
            self.visit_body(&self.current_body.clone());

            // Swap the root back.
            self.current_instance = previous_instance;
            self.current_body = previous_body;

            // Remove callee from the call stack.
            self.call_stack.pop();
        }
    }

    fn panic_and_dump_call_stack(&self, msg: &str) -> ! {
        const CALL_STACK_FILE: &str = "call_stack.log";
        fs::write(CALL_STACK_FILE, format!("{:#?}", self.call_stack))
            .expect("failed to save call stack before panicking");
        bug!("{msg}; wrote call stack to {CALL_STACK_FILE}");
    }
}

impl<'tcx> Visitor<'tcx> for RefinerVisitor<'tcx> {
    fn visit_terminator(&mut self, terminator: &Terminator<'tcx>, _location: Location) {
        match &terminator.kind {
            TerminatorKind::Call {
                func,
                args,
                fn_span,
                ..
            } => {
                self.refine_rec(func, args, *fn_span);
            }
            _ => {
                // TODO: visit other terminators, such as `Drop` or `Assert`.
            }
        }
    }
}

pub fn refine_from<'tcx>(
    root: Instance<'tcx>,
    reachable: FxHashSet<UsedMonoItem<'tcx>>,
    tcx: TyCtxt<'tcx>,
) -> RefinedUsageGraph<'tcx> {
    RefinerVisitor::new(root, reachable, tcx).refine()
}
