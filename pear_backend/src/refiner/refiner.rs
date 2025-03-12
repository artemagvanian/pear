use log::warn;
use std::fs;

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

use crate::{
    reachability::{ImplType, Usage, Node},
    refiner::utils::{fn_sig_eq_with_subtyping, is_intrinsic, is_virtual},
    serialize::{
        serialize_instance, serialize_instance_vec,
        serialize_refined_edges, serialize_span,
        serialize_transitive_refined_edges, 
        serialize_transitive_refined_vec
    },
    utils::{erase_regions_in_sig, fn_trait_method_sig},
};

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

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug, Serialize)]
pub struct TransitiveRefinedNode<'tcx> {
    #[serde(serialize_with = "serialize_instance")]
    node: Instance<'tcx>,
    #[serde(serialize_with = "serialize_span")]
    span: Span,
    is_refined: bool,
}

impl<'tcx> TransitiveRefinedNode<'tcx> {
    pub fn new(node: Instance<'tcx>, span: Span, taint: bool) -> Self {
        Self { node, span, is_refined: taint }
    }

    pub fn update_is_refined(&self, is_refined: bool) -> Self {
        Self {
            node: self.node,
            span: self.span,
            is_refined: is_refined,
        }
    }

    pub fn is_refined(&self) -> bool {
        self.is_refined
    }

    pub fn node(&self) -> Instance<'tcx> {
        self.node
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct TransitiveRefinedSubGraph<'tcx>  {
    // the child we build the subgraph up from, e.g., panic_fmt
    #[serde(serialize_with = "serialize_instance")]
    child_of_interest: Instance<'tcx>,

    // maps children to parents
    #[serde(serialize_with = "serialize_transitive_refined_edges")]
    backward_edges: FxHashMap<Instance<'tcx>, FxHashSet<TransitiveRefinedNode<'tcx>>>,

    #[serde(serialize_with = "serialize_transitive_refined_vec")]
    crate_boundaries: Vec<TransitiveRefinedNode<'tcx>>,
}

impl<'tcx> TransitiveRefinedSubGraph<'tcx> {
    fn new(child: Instance<'tcx>) -> Self {
        Self {
            child_of_interest: child,
            backward_edges: FxHashMap::default(),
            crate_boundaries: vec![]
        }
    }

    pub fn child_of_interest(&self) -> Instance<'tcx> {
        self.child_of_interest
    }

    pub fn crate_edges(&self) -> Vec<TransitiveRefinedNode<'tcx>> {
        self.crate_boundaries.clone()
    }

    fn add_edge(&mut self, child: &Instance<'tcx>, parent: &TransitiveRefinedNode<'tcx>) {
        self.backward_edges
            .entry(child.clone())
            .or_default()
            .insert(parent.clone());
    }
}

#[derive(Debug, Serialize)]
pub struct RefinedUsageGraph<'tcx> {
    #[serde(serialize_with = "serialize_instance")]
    root: Instance<'tcx>,

    // Maps every instance to the instances used by it.
    #[serde(serialize_with = "serialize_refined_edges")]
    forward_edges: FxHashMap<Instance<'tcx>, FxHashSet<RefinedNode<'tcx>>>,

    #[serde(skip_serializing)]
    backward_edges: FxHashMap<RefinedNode<'tcx>, FxHashSet<Instance<'tcx>>>,
}

impl<'tcx> RefinedUsageGraph<'tcx> {
    fn new(root: Instance<'tcx>) -> Self {
        Self {
            root,
            forward_edges: FxHashMap::default(),
            backward_edges: FxHashMap::default(),
        }
    }

    pub fn root(&self) -> Instance<'tcx> {
        self.root
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
        let mut instances = FxHashSet::from_iter([self.root]);
        for refined_nodes in self.forward_edges.values() {
            instances.extend(
                refined_nodes
                    .iter()
                    .flat_map(|refined_node| refined_node.instances()),
            );
        }
        instances
    }

    /// Returns a map of children to their parents (callers)
    /// such that the direct parents carry the refinement status of the child.
    fn precalculate_parents(&self) -> FxHashMap<Instance<'tcx>, Vec<TransitiveRefinedNode<'tcx>>> {
        let mut tainted_parents: FxHashMap<Instance<'tcx>, Vec<TransitiveRefinedNode<'tcx>>> = FxHashMap::default();
        for (refined_node, instances) in self.backward_edges.iter() {
            for child in refined_node.instances() {
                for parent in instances {
                    tainted_parents
                        .entry(child.clone())
                        .or_default()
                        .push(TransitiveRefinedNode::new(
                            parent.clone(),
                            refined_node.span(),
                            refined_node.is_refined(),
                        ));
                }
            }
        }
        tainted_parents
    }

    pub fn find_child_subgraph(
        &self,
        instance: &Instance<'tcx>,
        filter: &Vec<String>,
        tcx: TyCtxt<'tcx>,
    ) -> TransitiveRefinedSubGraph<'tcx> {
        let tainted_parents: FxHashMap<Instance<'tcx>, Vec<TransitiveRefinedNode<'tcx>>> = self.precalculate_parents(); 
        let mut subgraph = TransitiveRefinedSubGraph::new(*instance); 
        let mut stack = vec![];
        let mut visited = FxHashSet::default();
        self.find_child_subgraph_rec(
            instance,     
            &filter,
            tcx,
            false,
            &tainted_parents,
            &mut stack,
            &mut subgraph,
            &mut visited,
            None,
        );
        subgraph
    }

    fn find_child_subgraph_rec(
        &self,
        instance: &Instance<'tcx>,
        filter: &Vec<String>,
        tcx: TyCtxt<'tcx>,
        instance_refined: bool,
        tainted_parents: &FxHashMap<Instance<'tcx>, Vec<TransitiveRefinedNode<'tcx>>>,
        stack: &mut Vec<Instance<'tcx>>,
        subgraph: &mut TransitiveRefinedSubGraph<'tcx>, 
        visited: &mut FxHashSet<(Instance<'tcx>, bool, Option<TransitiveRefinedNode<'tcx>>)>, 
        crate_edge: Option<TransitiveRefinedNode<'tcx>>,
    ) {
        // skip if we've been to this instance
        if visited.contains(&(*instance, instance_refined, crate_edge)) {
            return;
        }
        // mark this instance as visited
        visited.insert((*instance, instance_refined, crate_edge));

        // don't recur into crates that are filtered
        if filter.iter().any(|filtered_item| {
            tcx.crate_name(instance.def_id().krate)
                .to_string()
                .contains(filtered_item)
        }) {
            return;
        }

        // TODO(corinn) filter whitelisted fns

        // `tainted_parents`` was precalculated. 
        // These TransitiveRefinedNodes already reflect the status of the child `instance`. 
        let parents: Vec<TransitiveRefinedNode<'tcx>> =
            tainted_parents.get(&instance).cloned().unwrap_or(vec![]);
        
        // Base case reached - at top-level function. 
        if parents.is_empty() {
            match crate_edge {
                Some(node) => 
                    // Update the crate_edge node because 
                    // there might have been something refined between top-level function and the crate boundary. 
                    // This would make it a prospective false positive from the POV of caller. 
                    subgraph.crate_boundaries.push(node.update_is_refined(instance_refined)),
                _ => {}
            }
        }
        for parent in parents {
            // Each precalculated parent already carries the refinement status of its direct child,  
            // but it may need to updated with the refinement status of a grandchild. 
            let updated_parent_status = parent.is_refined || instance_refined; 
            let updated_parent = parent.update_is_refined(updated_parent_status);
            // add the new edge to the subgraph
            subgraph.add_edge(&instance, &updated_parent); 
            // Once we hit a parent in the local crate, we store it as `crate_edge` and do not replace it again. 
            let crate_edge = crate_edge.or_else(|| { 
                if parent.node.def_id().is_local() { 
                    Some(parent)
                } else {
                    None
                }
            });
            
            if !stack.contains(&updated_parent.node) {
                stack.push(updated_parent.node);
                self.find_child_subgraph_rec(
                    &updated_parent.node,
                    filter,
                    tcx,
                    updated_parent.is_refined,
                    tainted_parents,
                    stack,
                    subgraph,
                    visited,
                    crate_edge
                );
                stack.pop();
            }
        }
    }

    /// Given an Instance of a child, 
    /// traverses up the RefinedUsageGraph to find the first in-crate callers
    pub fn find_reachable_edge_local_instances(
        &self,
        instance: Instance<'tcx>,
        filter: &Vec<String>,
        tcx: TyCtxt<'tcx>,
    ) -> Vec<TransitiveRefinedNode<'tcx>> {
        let subgraph = self.find_child_subgraph(&instance, filter, tcx); 
        subgraph.crate_boundaries 
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
    reachable_indirect: FxHashSet<Node<'tcx>>,
    refined_usage_graph: RefinedUsageGraph<'tcx>,
    call_stack: Vec<StackItem<'tcx>>,
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> RefinerVisitor<'tcx> {
    pub fn new(
        root: Instance<'tcx>,
        reachable: FxHashSet<Node<'tcx>>,
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
            refined_usage_graph: RefinedUsageGraph::new(root),
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
    reachable: FxHashSet<Node<'tcx>>,
    tcx: TyCtxt<'tcx>,
) -> RefinedUsageGraph<'tcx> {
    RefinerVisitor::new(root, reachable, tcx).refine()
}
