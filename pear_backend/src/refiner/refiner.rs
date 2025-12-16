use log::warn;
use std::{fs};

use rustc_hash::{FxHashMap, FxHashSet};
use rustc_hir::{def_id::DefId, LangItem};
use rustc_middle::{
    mir::{visit::Visitor, Body, Location, Terminator, TerminatorKind},
    ty::{
        self, EarlyBinder, FnSig, GenericArgsRef, Instance, InstanceKind, Ty, TyCtxt, TyKind, TypeFoldable, TypingEnv
    }
};
use rustc_public::mir::mono::InstanceDef; 
use rustc_span::{Span, DUMMY_SP};
use serde::Serialize;

extern crate unicode_segmentation;
use unicode_segmentation::UnicodeSegmentation;
use regex::Regex;

use crate::{
    reachability::{Node,  CollectedNode, CollectionReason, CallGraph},
    refiner::utils::{fn_sig_eq_with_subtyping, is_intrinsic, is_virtual},
    serialize::{
        serialize_instance, serialize_instance_vec, serialize_refined_edges, serialize_span,
        serialize_transitive_refined_edges, serialize_panic_dict, serialize_callers_and_spans
    },
    utils::{erase_regions_in_sig, fn_trait_method_sig},
};

type UsageGraph = CallGraph;
/* CallGraph maps Node -> CollectedNode

    UsageGraph maps MonoItem -> Node

*/
#[derive(Clone, Serialize)]
pub struct PanicDict<'tcx> {
    #[serde(serialize_with = "serialize_panic_dict")]
    //pub panic_dict: FxHashMap<Instance<'tcx>, (bool, String, FxHashSet<Instance<'tcx>>)>,
    pub panic_dict: FxHashMap<Instance<'tcx>, PanicEntry<'tcx>>,
}

#[derive(Clone, Serialize)]
pub struct PanicEntry<'tcx> {
    #[serde(serialize_with = "serialize_callers_and_spans")]
    callers_and_spans: FxHashSet<(Instance<'tcx>, Span)>,
    is_in_crate: bool,
    is_in_stdlib: bool,
    has_documented_panic: bool, 
    doc_str: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize)]
pub enum RefinedNode<'tcx> {
    Concrete {
        #[serde(serialize_with = "serialize_instance")]
        instance: Instance<'tcx>,
        #[serde(serialize_with = "serialize_span")]
        span: Span,
        #[serde(serialize_with = "serialize_span")]
        terminator_span: Span,
    },
    Refined {
        #[serde(serialize_with = "serialize_instance_vec")]
        instances: Vec<Instance<'tcx>>,
        #[serde(serialize_with = "serialize_span")]
        span: Span,
        #[serde(serialize_with = "serialize_span")]
        terminator_span: Span,
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

    pub fn terminator_span(&self) -> Span {
        match self {
            Self::Concrete {
                terminator_span, ..
            }
            | Self::Refined {
                terminator_span, ..
            } => *terminator_span,
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
        Self {
            node,
            span,
            is_refined: taint,
        }
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
pub struct TransitiveRefinedSubGraph<'tcx> {
    // The child we build the subgraph up from, e.g., panic_fmt.
    #[serde(serialize_with = "serialize_instance")]
    child_of_interest: Instance<'tcx>,
    #[serde(serialize_with = "serialize_instance")]
    root_node: Instance<'tcx>,

    // Maps children to parents.
    #[serde(serialize_with = "serialize_transitive_refined_edges")]
    backward_edges: FxHashMap<Instance<'tcx>, FxHashSet<TransitiveRefinedNode<'tcx>>>,
    // Maps parents to children
    #[serde(skip_serializing)]
    forward_edges: FxHashMap<Instance<'tcx>, FxHashSet<Instance<'tcx>>>,
    crate_boundaries: Vec<TransitiveRefinedNode<'tcx>>,
}

impl<'tcx> TransitiveRefinedSubGraph<'tcx> {
    fn new(child: Instance<'tcx>) -> Self {
        Self {
            child_of_interest: child,
            root_node: child,
            backward_edges: FxHashMap::default(),
            forward_edges: FxHashMap::default(),
            crate_boundaries: vec![],
        }
    }

    pub fn child_of_interest(&self) -> Instance<'tcx> {
        self.child_of_interest
    }

    pub fn crate_boundaries(&self) -> Vec<TransitiveRefinedNode<'tcx>> {
        self.crate_boundaries.clone()
    }

    pub fn is_empty(&self) -> bool {
        return self.backward_edges.get(&self.child_of_interest).is_none() ||
        self.forward_edges.get(&self.root_node).is_none()
    }

    fn is_circular(&self, 
        parents: &FxHashSet<TransitiveRefinedNode<'tcx>>, 
        instance: &Instance<'tcx>,
    ) -> bool {
        for par in parents.iter() {
            if par.node() != *instance {
                return false 
            }
        }
        return true
    }

    fn is_local(&self, instance: &Instance<'tcx>) -> bool {
        for local in self.crate_boundaries.iter() {
            if local.node() == *instance {
                return true
            }
        }
        return false
    }

    fn find_reachable(&mut self, 
        can_reach: &mut FxHashSet<Instance<'tcx>>,
        curr_node: &Instance<'tcx>,
    ) {
        if *curr_node == self.child_of_interest() { return; }
        
        let children: FxHashSet<Instance> = match self.forward_edges.get(&curr_node) {
            Some(map) => map.clone(),
            None => FxHashSet::default()
        };

        children.iter().for_each(|child| {
            if child != curr_node && !can_reach.contains(child) {
                can_reach.insert(*child);
                self.find_reachable(can_reach, child);
            }
        });
    }

    pub fn cleanup_unreachable(&mut self
    ) {
        let mut can_reach: FxHashSet<Instance> = FxHashSet::default();
        self.find_reachable(&mut can_reach, &self.root_node.clone());
        can_reach.insert(self.root_node.clone());

        let mut nodes = FxHashSet::default();

        self.backward_edges.clone().into_keys().for_each(|node|
            { nodes.insert(node); });
        self.forward_edges.clone().into_keys().for_each(|node|
            { nodes.insert(node); });

        for node in nodes {
            if !can_reach.contains(&node) {
                self.remove_node(&node);
            }
        }
    }

    /*
    checks if the given instance is in stdlib, 
    including if the instance is a fn from an impl of a trait
    defined in stdlib
     */
    fn is_in_stdlib(&mut self, 
        instance: &Instance<'tcx>, 
        tcx: TyCtxt,
    ) -> bool {
        let mut path = tcx.def_path_str(instance.def_id());

        // preserve only fn path to be implemented (so we can allowlist for all impls of the same fn)
        if path.contains(" as ") {
            let imp: Option<(_, _)> = path.rsplit_once("as");
            match imp {
                Some((_, str_two)) => path = str_two.to_string(),
                None => ()
            };
        }

        let tokens = path.unicode_words().collect::<Vec<&str>>();
        if ((tokens[0] == "std") || (tokens[0] == "alloc") || (tokens[0] == "core")) 
        && !self.is_local(instance) { return true };

        false
    }

    fn has_documented_panic(&mut self, 
        instance: &Instance,
        tcx: TyCtxt,
    ) -> (bool, String) {
        let panic_re = Regex::new(r"# Panics").expect("Regex failure");
        let panic_full_re = Regex::new(r"(# Panics\s.*)(#)").expect("Regex failure");
        let panic_full_end_re = Regex::new(r"# Panics\s.*").expect("Regex failure");
        let attrs = tcx.get_all_attrs(instance.def_id());
        let mut doc_string = String::from("");
        for attr in attrs {
            if let Some(str) = attr.doc_str() {
                doc_string.push_str(str.as_str());
            }
        }

        if panic_re.is_match(&doc_string) {
            let str = panic_full_re.captures(&doc_string);
            if str.is_some() {
                return (true, str.unwrap()[1].to_string())
            } else {
                let panic_str = panic_full_end_re.find(&doc_string).unwrap();
                return (true, panic_str.as_str().to_string());
            }
        }
        return (false, String::from(""))
    }

    pub fn build_panic_dict(&mut self, 
        panics: &mut PanicDict<'tcx>,
        tcx: TyCtxt,
    ) {
        // add if in stdlib
        let mut immediate_children: FxHashSet<Instance> = FxHashSet::default();
        self.crate_boundaries().iter().for_each(|caller| 
            immediate_children.extend(self.forward_edges.get(&caller.node()).unwrap().clone()));
        
        // get first not-in-crate paths
        for child in immediate_children {
            for par in self.backward_edges.get(&child).unwrap().clone().iter() {
                if self.crate_boundaries().contains(&par) {
                    if let Some(entry) = panics.panic_dict.get_mut(&child) {
                        entry.callers_and_spans.insert((par.node(), par.span()));
                    } else {
                        let (doc_panic, doc_str) = self.has_documented_panic(&child, tcx);
                        let mut entry = PanicEntry{
                            callers_and_spans: FxHashSet::default(),
                            is_in_crate: child.def_id().is_local(),
                            is_in_stdlib: self.is_in_stdlib(&child, tcx),
                            has_documented_panic: doc_panic,
                            doc_str: doc_str,
                        };

                        entry.callers_and_spans.insert((par.node(), par.span()));
                        panics.panic_dict.insert(child, entry);
                    }
                }
            };
        }
    }

    fn remove_node(&mut self, 
        instance: &Instance<'tcx>,
    ) {
        self.backward_edges.remove(&instance);
        // do check for children
        let mut children: FxHashSet<Instance<'tcx>> = FxHashSet::default();
        if let Some(child_nodes) = self.forward_edges.get(&instance) {
            children = child_nodes.clone();
        }

        children.iter().for_each(|child| {
            if let Some(set) = self.backward_edges.get(&child) {
                let new_set = FxHashSet::from_iter(set.iter().filter(|&node| node.node() != *instance).cloned());
                self.backward_edges.insert(*child, new_set);                       
            }
        });
        self.forward_edges.remove(instance);
    }

    pub fn allowlist_stdlib(&mut self,         
        tcx: TyCtxt<'tcx>,
        head: &Instance<'tcx>,
        re: Result<Regex, regex::Error>,
        depth: u32,
    ){ 
        let panic_re = Regex::new("panic").unwrap();
        let path = tcx.def_path_str(head.def_id());
        let tokens = path.unicode_words().collect::<Vec<&str>>();

        let children: FxHashSet<Instance> = match self.forward_edges.get(&head) {
            Some(map) => {map.clone()}
            None => {FxHashSet::default()}
        };

        if panic_re.find(path.as_str()).is_some() { return; } // don't allowlist, i.e., panic_fmt

        if ((tokens[0] == "std") || (tokens[0] == "alloc") || (tokens[0] == "core")) && !self.is_local(head){
            // we've hit a stdlib fn with no documented panic -> remove the whole call chain 
            if !self.has_documented_panic(head, tcx).0 && *head != self.child_of_interest {
                self.remove_node_upwards(tcx, head, );
            } else { 
                return; } // we've hit a stdlib fn with a documented panic -> stop checking children
            
        } else if depth > 20 {
            return;
        } else {
            children.iter().for_each(|child: &_| {
                self.allowlist_stdlib(tcx, child, re.clone(), depth + 1);
            });
        }
    }

    fn add_edge(&mut self, child: &Instance<'tcx>, parent: &TransitiveRefinedNode<'tcx>) {
        
        self.backward_edges
            .entry(child.clone())
            .or_default()
            .insert(parent.clone());

        self.forward_edges
            .entry(parent.node().clone())
            .or_default()
            .insert(child.clone());
    }

    fn cleanup_crate_bounds(&mut self) {

        let mut new_boundaries: Vec<TransitiveRefinedNode<'tcx>> = Vec::default();
        
        self.crate_boundaries.iter().for_each(|bound| {
            for value_set in self.backward_edges.values() {
                if value_set.contains(bound) {
                    new_boundaries.push(*bound);
                    break;};
            }
        });

        self.crate_boundaries = new_boundaries;
    }

    fn remove_node_upwards(&mut self, 
        tcx: TyCtxt<'tcx>, 
        instance: &Instance<'tcx>, 
    ) {
        let mut children: FxHashSet<Instance> = FxHashSet::default();
        if let Some(child_nodes) = self.forward_edges.get(&instance) {
            children = child_nodes.clone();
        }

        let mut parents: FxHashSet<TransitiveRefinedNode> = FxHashSet::default();
        if let Some(parent_nodes) = self.backward_edges.get(&instance) {
            parents = parent_nodes.clone();
        }

        self.backward_edges.remove(instance);
        self.forward_edges.remove(instance);

        children.iter().for_each(|child| {
            if let Some(set) = self.backward_edges.get(child) {
                let new_set = FxHashSet::from_iter(set.iter().filter(|&node| node.node() != *instance).cloned());  
            
                if new_set.is_empty() || self.is_circular(&parents, instance) {self.backward_edges.remove(child);}
                else {self.backward_edges.insert(*child, new_set); }
            }
        });

        parents.iter().for_each(|parent| {
            if let Some(set) = self.forward_edges.get(&parent.node) {
                let new_set = FxHashSet::from_iter(set.iter().filter(|&node| *node != *instance).cloned());
                self.forward_edges.insert(parent.node, new_set.clone());
                
                if new_set.is_empty() {
                    self.remove_node_upwards(tcx, &parent.node())};
            }
        });
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

    pub fn get_forward_edges(&self, instance: &Instance<'tcx>) -> FxHashSet<RefinedNode<'tcx>> {
        self.forward_edges
            .get(instance)
            .cloned()
            .unwrap_or_default()
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

    /// Returns a map of children to their parents (callers) such that the direct parents carry the
    /// refinement status of the child.
    fn precalculate_parents(&self) -> FxHashMap<Instance<'tcx>, Vec<TransitiveRefinedNode<'tcx>>> {
        let mut tainted_parents: FxHashMap<Instance<'tcx>, Vec<TransitiveRefinedNode<'tcx>>> =
            FxHashMap::default();
        for (refined_node, instances) in self.backward_edges.iter() {
            for child in refined_node.instances() {
                for parent in instances {
                    tainted_parents.entry(child.clone()).or_default().push(
                        TransitiveRefinedNode::new(
                            parent.clone(),
                            refined_node.span(),
                            refined_node.is_refined(),
                        ),
                    );
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
        allow_std: bool,
    ) -> TransitiveRefinedSubGraph<'tcx> {
        let tainted_parents: FxHashMap<Instance<'tcx>, Vec<TransitiveRefinedNode<'tcx>>> =
            self.precalculate_parents();
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
            allow_std
        );

        let root = &subgraph.root_node.clone();
        let reg: Result<Regex, regex::Error> = Regex::new("# Panics");
        if allow_std { 
            subgraph.allowlist_stdlib(tcx, root, reg, 0);
        }
        
        subgraph.cleanup_unreachable();
        subgraph.cleanup_crate_bounds();
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
        allow_std: bool,
    ) {
        // Skip if we've been to this instance.
        if visited.contains(&(*instance, instance_refined, crate_edge)) {
            return;
        }

        // Mark this instance as visited.
        visited.insert((*instance, instance_refined, crate_edge));
        let mut path = tcx.def_path_str(instance.def_id());

        // preserve only fn path to be implemented (so we can allowlist for all impls of the same fn)
        if path.contains("as") {
            let imp: Option<(_, _)> = path.rsplit_once("as");
            match imp {
                Some((_, str_two)) => path = str_two.to_string(),
                None => ()
            };
        }

        let tokens = path.unicode_words().collect::<Vec<&str>>();
     
        // compare prefixes  - do without streams!! (aka no unicode_words)
        for val in filter.iter() {
            let allowed_tokens: Vec<&str> = val.unicode_words().collect::<Vec<&str>>();
            let matching: usize = allowed_tokens.iter()
                .zip(tokens.iter())
                .filter(|(allowed_tokens, tokens)| allowed_tokens == tokens)
                .count();
            if matching == allowed_tokens.len() {
                return;
            }
        }

        // Don't recur into crates that are filtered.
        if filter.iter().any(|filtered_item: &String| {
            // Match filtered item's crate.
            tcx.crate_name(instance.def_id().krate)
                .to_string()
                .contains(filtered_item)
        }) {
            return;
        }

        // Since we have precalculated `tainted_parents`, these nodes already reflect the status of
        // the child's instance.
        let parents: Vec<TransitiveRefinedNode<'tcx>> =
            tainted_parents.get(&instance).cloned().unwrap_or(vec![]);
        
        // Base case reached - at top-level function.
        if parents.is_empty() {
            match crate_edge {
                Some(node) =>
                // Update the crate boundary node because there might have been something
                // refined between top-level function and the crate boundary. This would make it
                // a prospective false positive from the POV of caller.
                {
                    subgraph
                        .crate_boundaries
                        .push(node.update_is_refined(instance_refined))
                }
                _ => {}
            }
            subgraph.root_node = *instance;
        }

        for parent in parents {
            // Each precalculated parent already carries the refinement status of its direct child,
            // but it may need to updated with the refinement status of a grandchild.
            let updated_parent_status = parent.is_refined || instance_refined;
            let updated_parent: TransitiveRefinedNode<'_> = parent.update_is_refined(updated_parent_status);
            // Add the new edge to the subgraph.

            subgraph.add_edge(&instance, &updated_parent);

            // Once we hit a parent in the local crate, we store it and do not replace it again.
            let crate_edge: Option<TransitiveRefinedNode<'tcx>> = crate_edge.or_else(|| {
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
                    crate_edge,
                    allow_std,
                );
                stack.pop();
            }
        }
    }

    /// Given an instance of a child, traverses up the graph to find the first in-crate callers.
    pub fn find_reachable_edge_local_instances(
        &self,
        instance: Instance<'tcx>,
        filter: &Vec<String>,
        tcx: TyCtxt<'tcx>,
        allow_std: bool,
        panic_dict: &mut PanicDict<'tcx>,
    ) -> Vec<TransitiveRefinedNode<'tcx>> {
        let mut subgraph: TransitiveRefinedSubGraph<'_> = self.find_child_subgraph(&instance, filter, tcx, allow_std);
        subgraph.build_panic_dict(panic_dict, tcx);
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
    reachable_indirect: Vec<CollectedNode>,
    refined_usage_graph: RefinedUsageGraph<'tcx>,
    call_stack: Vec<StackItem<'tcx>>,
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> RefinerVisitor<'tcx> {
    pub fn new(root: Instance<'tcx>, reachable: Vec<CollectedNode>, tcx: TyCtxt<'tcx>) -> Self {
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
                match reachable_indirect.reason() {
                    CollectionReason::Static
                    | CollectionReason:: {
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
            .filter(|reachable_indirect| match reachable_indirect.reason() {
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
            .filter(|reachable_indirect| match reachable_indirect.reason() {
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
        v: rustc_type_ir::EarlyBinder<TyCtxt<'tcx>, T>, 
    ) {
        self.current_instance
            .instantiate_mir_and_normalize_erasing_regions(self.tcx, TypingEnv::fully_monomorphized(), v)
    }

    fn refine_rec(&mut self, fn_ty: Ty<'tcx>, span: Span, terminator_span: Span) {
        // Refine the passed function operand.
        let fn_ty = self.instantiate_with_current_instance(EarlyBinder::bind(fn_ty));

        let refined = match fn_ty.kind().clone() {
            TyKind::FnDef(def_id, generic_args) => {
                let instance: Instance<'_> = ty::Instance::expect_resolve(
                    self.tcx,
                    TypingEnv::fully_monomorphized(),
                    def_id,
                    generic_args,
                    self.tcx.def_span(def_id)
                );
                match instance.def { // of type InstanceKind
                    InstanceKind::Virtual(method_def_id, ..) => RefinedNode::Refined {
                        instances: self.candidates_for_virtual(method_def_id, instance.args),
                        span,
                        terminator_span,
                    },
                    _ => RefinedNode::Concrete {
                        instance,
                        span,
                        terminator_span,
                    },
                }
            }
            // TyKind::FnPtr(ty::Binder<I, FnSigTys<TyCtxt>>, FnHeader<TyCtxt<'_>>>)
            TyKind::FnPtr(binder, fn_header) => {
                // type PolyFnSig is alias of Binder<'tcx, FnSig<'tcx>>;
                let poly_fn_sig = binder.with(fn_header);
                let fn_sig: rustc_type_ir::FnSig<TyCtxt<'_>> = erase_regions_in_sig(poly_fn_sig, self.tcx);
                RefinedNode::Refined {
                    instances: self.candidates_for_fn_ptr(fn_sig),
                    span,
                    terminator_span,
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
    fn visit_terminator(&mut self, terminator: &Terminator<'tcx>, location: Location) {
        let terminator_span = terminator.source_info.span;
        match &terminator.kind {
            TerminatorKind::Call { func, fn_span, .. } => {
                self.refine_rec(
                    func.ty(&self.current_body, self.tcx),
                    *fn_span,
                    terminator_span,
                );
            }
            TerminatorKind::Drop { ref place, .. } => {
                let ty = place.ty(&self.current_body, self.tcx).ty;
                let def_id = self.tcx.require_lang_item(LangItem::DropInPlace, DUMMY_SP); // span only used to emit error
                let args = self.tcx.mk_args(&[ty.into()]);
                self.refine_rec(
                    self.tcx.type_of(def_id).instantiate(self.tcx, args),
                    DUMMY_SP,
                    terminator_span,
                );
            }
            _ => {
                // TODO: visit other terminators, such as `Assert`.
            }
        }
        self.super_terminator(terminator, location);
    }
}

pub fn refine_from<'tcx>(
    root: Instance<'tcx>,
    reachable: FxHashSet<Node<'tcx>>,
    tcx: TyCtxt<'tcx>,
) -> RefinedUsageGraph<'tcx> {
    RefinerVisitor::new(root, reachable, tcx).refine()
}