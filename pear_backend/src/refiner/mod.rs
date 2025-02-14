use log::warn;
use std::fs;
use utils::fn_sig_eq_with_subtyping;

use rustc_hash::{FxHashMap, FxHashSet};
use rustc_hir::def_id::DefId;
use rustc_middle::{
    mir::{visit::Visitor, Body, Location, Operand, Terminator, TerminatorKind},
    ty::{
        self, EarlyBinder, FnSig, GenericArgsRef, Instance, InstanceDef, ParamEnv, TyCtxt, TyKind,
        TypeFoldable,
    },
};
use rustc_span::Span;
use serde::Serialize;

use crate::reachability::{ImplType, Usage, UsedMonoItem};
use crate::serialize::{
    serialize_instance, serialize_instance_vec, serialize_refined_edges, serialize_span,
};
use crate::utils::erase_regions_in_sig;
use crate::{
    refiner::utils::{is_intrinsic, is_virtual},
    utils::fn_trait_method_sig,
};

mod utils;

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
}

impl<'tcx> RefinedNode<'tcx> {
    pub fn instances(&self) -> Vec<Instance<'tcx>> {
        match self {
            RefinedNode::Concrete { instance, .. } => vec![instance.clone()],
            RefinedNode::Refined { instances, .. } => instances.clone(),
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::Concrete { span, .. } | Self::Refined { span, .. } => *span,
        }
    }

    pub fn is_refined(&self) -> bool {
        matches!(self, RefinedNode::Refined { .. })
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct TaintedNode<'tcx> {
    node: Instance<'tcx>,
    span: Span,
    taint: bool,
}

impl<'tcx> TaintedNode<'tcx> {
    pub fn new(node: Instance<'tcx>, span: Span, taint: bool) -> Self {
        Self { node, span, taint }
    }

    pub fn retaint(&self, new_taint: bool) -> Self {
        Self {
            node: self.node,
            span: self.span,
            taint: new_taint,
        }
    }

    pub fn is_tainted(&self) -> bool {
        self.taint
    }

    pub fn node(&self) -> Instance<'tcx> {
        self.node
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Serialize)]
pub struct RefinedUsageGraph<'tcx> {
    // Maps every instance to the instances used by it.
    #[serde(serialize_with = "serialize_refined_edges")]
    forward_edges: FxHashMap<Instance<'tcx>, FxHashSet<RefinedNode<'tcx>>>,

    #[serde(skip_serializing)]
    backward_edges: FxHashMap<RefinedNode<'tcx>, FxHashSet<Instance<'tcx>>>,
}

impl<'tcx> RefinedUsageGraph<'tcx> {
    fn new() -> Self {
        Self {
            forward_edges: FxHashMap::default(),
            backward_edges: FxHashMap::default(),
        }
    }

    fn add_edge(&mut self, from: &Instance<'tcx>, to: &RefinedNode<'tcx>) {
        self.forward_edges
            .entry(from.clone())
            .or_default()
            .insert(to.clone());

        self.backward_edges
            .entry(to.clone())
            .or_default()
            .insert(from.clone());
    }

    pub fn instances(&self) -> FxHashSet<Instance<'tcx>> {
        let mut instances = FxHashSet::default();
        for (from, to) in self.forward_edges.iter() {
            instances.insert(from.clone());
            instances.extend(to.iter().flat_map(|refined_node| refined_node.instances()));
        }
        instances
    }

    pub fn find_reachable_edge_local_instances(
        &self,
        instance: Instance<'tcx>,
        filter: &Vec<String>,
        tcx: TyCtxt<'tcx>,
    ) -> Vec<TaintedNode<'tcx>> {
        // Precalculate the inverse of the backward edges.
        let mut tainted_parents: FxHashMap<Instance<'tcx>, Vec<TaintedNode<'tcx>>> =
            FxHashMap::default();
        for (refined_node, instances) in self.backward_edges.iter() {
            for child in refined_node.instances() {
                for parent in instances {
                    tainted_parents
                        .entry(child.clone())
                        .or_default()
                        // .push(respan(refined_node.span(), parent.clone()));
                        .push(TaintedNode::new(
                            parent.clone(),
                            refined_node.span(),
                            refined_node.is_refined(),
                        ));
                }
            }
        }

        let mut result = vec![];
        let mut stack = vec![];
        let mut visited = FxHashSet::default();
        self.find_reachable_edge_local_instances_rec(
            instance,
            &filter,
            tcx,
            false,
            &tainted_parents,
            &mut stack,
            &mut result,
            &mut visited,
            None,
        );

        result
    }

    fn find_reachable_edge_local_instances_rec(
        &self,
        instance: Instance<'tcx>,
        filter: &Vec<String>,
        tcx: TyCtxt<'tcx>,
        instance_tainted: bool,
        tainted_parents: &FxHashMap<Instance<'tcx>, Vec<TaintedNode<'tcx>>>,
        stack: &mut Vec<Instance<'tcx>>,
        result: &mut Vec<TaintedNode<'tcx>>,
        visited: &mut FxHashSet<(Instance<'tcx>, bool, Option<TaintedNode<'tcx>>)>,
        crate_edge: Option<TaintedNode<'tcx>>,
    ) {
        if visited.contains(&(instance, instance_tainted, crate_edge)) {
            return;
        }
        visited.insert((instance, instance_tainted, crate_edge));

        if filter.iter().any(|filtered_item| {
            tcx.crate_name(instance.def_id().krate)
                .to_string()
                .contains(filtered_item)
        }) {
            return;
        }

        let parents: Vec<TaintedNode<'tcx>> =
            tainted_parents.get(&instance).cloned().unwrap_or(vec![]);

        if parents.is_empty() {
            match crate_edge {
                Some(tainted_node) => result.push(tainted_node.retaint(instance_tainted)),
                _ => {}
            }
        }

        for parent in parents {
            let crate_edge = crate_edge.or_else(|| {
                if parent.node.def_id().is_local() {
                    Some(parent)
                } else {
                    None
                }
            });

            if !stack.contains(&parent.node) {
                stack.push(parent.node);
                self.find_reachable_edge_local_instances_rec(
                    parent.node,
                    filter,
                    tcx,
                    instance_tainted || parent.is_tainted(),
                    tainted_parents,
                    stack,
                    result,
                    visited,
                    crate_edge,
                );
                stack.pop();
            }
        }
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

    /// Given a signature for a function pointer, find all indirectly collected functions that have
    /// this signature.
    fn candidates_for_fn_ptr(&self, ambiguous_fn_sig: FnSig<'tcx>) -> Vec<Instance<'tcx>> {
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
                        if fn_sig_eq_with_subtyping(ambiguous_fn_sig, indirect_fn_sig) {
                            Some(reachable_indirect.expect_instance())
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
            .collect();

        if refined_candidates.is_empty() {
            warn!("found no refined instances for function pointer with signature = {ambiguous_fn_sig:#?}",);
        }

        refined_candidates
    }

    fn candidates_for_vtable_call(
        &self,
        virtual_method_def_id: DefId,
        virtual_args: GenericArgsRef<'tcx>,
    ) -> Vec<Instance<'tcx>> {
        let refined_candidates: Vec<Instance<'tcx>> = self
            .reachable_indirect
            .iter()
            .filter(|reachable_indirect| match reachable_indirect.usage() {
                Usage::VtableItem { impl_type, .. } => {
                    let possible_instance = reachable_indirect.expect_instance();
                    match impl_type {
                        ImplType::Explicit {
                            def_id: impl_def_id,
                        } => self
                            .tcx
                            .impl_item_implementor_ids(impl_def_id)
                            .get(&virtual_method_def_id)
                            .map(|impl_method_def_id| {
                                *impl_method_def_id == possible_instance.def_id()
                            })
                            .unwrap_or(false),
                        ImplType::Inherent => virtual_method_def_id == possible_instance.def_id(),
                    }
                }
                _ => false,
            })
            .map(|used_mono_item| used_mono_item.expect_instance())
            .collect();

        if refined_candidates.is_empty() {
            warn!(
                "found no refined instances for a vtable method with def_id = {virtual_method_def_id:#?}, args = {virtual_args:#?}"
            );
        }

        refined_candidates
    }

    fn candidates_for_fn_trait_call(
        &self,
        virtual_method_def_id: DefId,
        virtual_args: GenericArgsRef<'tcx>,
    ) -> Vec<Instance<'tcx>> {
        let indirect_sig = fn_trait_method_sig(virtual_method_def_id, virtual_args, self.tcx);
        let refined_candidates: Vec<Instance<'tcx>> = self
            .reachable_indirect
            .iter()
            .filter(|reachable_indirect| match reachable_indirect.usage() {
                Usage::FnTraitItem { sig } => indirect_sig == sig,
                _ => false,
            })
            .map(|used_mono_item| used_mono_item.expect_instance())
            .collect();

        if refined_candidates.is_empty() {
            warn!(
                "found no refined instances for a vtable method with def_id = {virtual_method_def_id:#?}, args = {virtual_args:#?}"
            );
        }

        refined_candidates
    }

    /// Given a def_id of a virtual method, find all indirectly collected vtable items that
    /// implement this method.
    fn candidates_for_virtual(
        &self,
        virtual_method_def_id: DefId,
        virtual_args: GenericArgsRef<'tcx>,
    ) -> Vec<Instance<'tcx>> {
        if self.tcx.is_fn_trait(self.tcx.parent(virtual_method_def_id)) {
            self.candidates_for_fn_trait_call(virtual_method_def_id, virtual_args)
        } else {
            self.candidates_for_vtable_call(virtual_method_def_id, virtual_args)
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
                    InstanceDef::Virtual(method_def_id, ..) => RefinedNode::Refined {
                        instances: self.candidates_for_virtual(method_def_id, instance.args),
                        span,
                    },
                    _ => RefinedNode::Concrete { instance, span },
                }
            }
            TyKind::FnPtr(poly_fn_sig) => {
                let fn_sig = erase_regions_in_sig(poly_fn_sig, self.tcx);
                RefinedNode::Refined {
                    instances: self.candidates_for_fn_ptr(fn_sig),
                    span,
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
            .add_edge(&self.current_instance, &refined);

        for callee in refined.instances() {
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
