use rustc_hash::{FxHashMap, FxHashSet};
use rustc_middle::{
    mir::{visit::Visitor, Body, Location, Operand, Terminator, TerminatorKind},
    ty::{
        self, normalize_erasing_regions::NormalizationError, EarlyBinder, FnSig, Instance,
        ParamEnv, TyCtxt, TyKind, TypeFoldable,
    },
};
use serde::{ser::SerializeStruct, Serialize};

use crate::reachability::UsedMonoItem;

mod utils;
use utils::*;

pub use utils::contains_non_concrete_type;

#[derive(Debug)]
pub struct RefinedUsageGraph<'tcx> {
    // Maps every instance to the instances used by it.
    forward_edges: FxHashMap<Instance<'tcx>, FxHashSet<Instance<'tcx>>>,
}

impl<'tcx> Serialize for RefinedUsageGraph<'tcx> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("UsageMap", 2)?;

        let forward_edges: FxHashMap<_, _> = self
            .forward_edges
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    v.iter().map(|v| v.to_string()).collect::<FxHashSet<_>>(),
                )
            })
            .collect();

        state.serialize_field("forward_edges", &forward_edges)?;
        state.end()
    }
}

impl<'tcx> RefinedUsageGraph<'tcx> {
    fn new() -> Self {
        Self {
            forward_edges: FxHashMap::default(),
        }
    }

    fn add_edge(&mut self, from: Instance<'tcx>, to: Instance<'tcx>) {
        self.forward_edges.entry(from).or_default().insert(to);
    }
}

pub struct RefinerVisitor<'tcx> {
    current_instance: Instance<'tcx>,
    current_body: Body<'tcx>,
    reachable_indirect: FxHashSet<Instance<'tcx>>,
    refined_usage_graph: RefinedUsageGraph<'tcx>,
    call_stack: Vec<Instance<'tcx>>,
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
        let root_body = tcx.instance_mir(root.def).to_owned();

        // Find all reachable mono items that were not used directly, they will be used when
        // resolving ambiguous calls.
        let reachable_indirect = reachable
            .iter()
            .filter_map(|used_mono_item| {
                used_mono_item
                    .instance_if_callable(tcx)
                    .filter(|_| used_mono_item.is_indirect())
            })
            .collect();

        Self {
            current_instance: root,
            current_body: root_body,
            reachable_indirect,
            refined_usage_graph: RefinedUsageGraph::new(),
            call_stack: vec![root],
            tcx,
        }
    }

    pub fn refine(mut self) -> RefinedUsageGraph<'tcx> {
        self.visit_body(&self.current_body.clone());
        self.refined_usage_graph
    }

    fn candidates_for_ambiguous(&self, ambiguous_fn_sig: FnSig<'tcx>) -> Vec<Instance<'tcx>> {
        // Check whether a reachable indirect item could be used to resolve the ambiguous one.
        let maybe_refined_candidates = self
            .reachable_indirect
            .iter()
            .filter(|reachable_indirect| {
                // Try instantiating the signature of an instance with generic args in scope.
                let indirect_fn_ty = instance_type(reachable_indirect, self.tcx);
                let indirect_fn_sig = normalized_fn_sig(indirect_fn_ty, self.tcx);
                is_resolution_of(ambiguous_fn_sig, indirect_fn_sig, self.tcx)
            })
            .cloned()
            .collect::<Vec<_>>();

        if maybe_refined_candidates.is_empty() {
            // TODO: record an ambiguous instance.
            bug!(
                "did not find any suitable candidates for resolution, call_stack={:#?}",
                self.call_stack
            );
        }
        maybe_refined_candidates
    }

    fn try_instantiate_with_current_instance<T: TypeFoldable<TyCtxt<'tcx>>>(
        &self,
        v: EarlyBinder<T>,
    ) -> Result<T, NormalizationError<'tcx>> {
        self.current_instance
            .try_instantiate_mir_and_normalize_erasing_regions(self.tcx, ParamEnv::reveal_all(), v)
    }

    fn refine_rec(&mut self, func: &Operand<'tcx>, _args: &Vec<Operand<'tcx>>) {
        // Refine the passed function operand.
        let function_ty = func.ty(&self.current_body, self.tcx);
        let Ok(function_ty) =
            self.try_instantiate_with_current_instance(EarlyBinder::bind(function_ty))
        else {
            // TODO: record an irresolvable instance.
            bug!(
                "failed to resolve the instance during refinement, call_stack={:#?}",
                self.call_stack
            );
        };

        let fn_sig = normalized_fn_sig(function_ty, self.tcx);
        let callees = match function_ty.kind().to_owned() {
            TyKind::FnDef(def_id, generic_args) => {
                if let Ok(Some(instance)) =
                    ty::Instance::resolve(self.tcx, ParamEnv::reveal_all(), def_id, generic_args)
                    && !is_virtual(instance)
                {
                    vec![instance]
                } else {
                    self.candidates_for_ambiguous(fn_sig)
                }
            }
            TyKind::FnPtr(..) => self.candidates_for_ambiguous(fn_sig),
            _ => bug!(
                "unexpected callee type encountered when performing refinement, call_stack={:#?}",
                self.call_stack
            ),
        };

        for callee in callees.into_iter() {
            // Skip the function if it is already in the usage graph.
            if self
                .refined_usage_graph
                .forward_edges
                .get(&self.current_instance)
                .is_some_and(|s| s.contains(&callee))
            {
                continue;
            }

            // Add the edge to the refined graph.
            self.refined_usage_graph
                .add_edge(self.current_instance, callee);

            // Resolved callee should not be virtual.
            assert!(
                !is_virtual(callee),
                "resolved to a virtual callee when performing refinement, call_stack={:#?}",
                self.call_stack
            );

            // Skip recurring into the item if the item does not have a body.
            if self.tcx.is_foreign_item(callee.def_id()) || is_intrinsic(callee) {
                continue;
            }

            // We do not instantiate and normalize body just yet but do it lazily instead to support
            // partially parametric instances.
            let callee_body = self.tcx.instance_mir(callee.def).to_owned();

            // Save previous instance and previous body to swap in later.
            let previous_instance = self.current_instance;
            let previous_body = self.current_body.clone();

            // Swap root & body for the refined instance.
            self.current_instance = callee;
            self.current_body = callee_body;

            // Add callee to the call stack.
            self.call_stack.push(callee);

            // Continue collection.
            self.visit_body(&self.current_body.clone());

            // Swap the root back.
            self.current_instance = previous_instance;
            self.current_body = previous_body;

            // Remove callee from the call stack.
            self.call_stack.pop();
        }
    }
}

impl<'tcx> Visitor<'tcx> for RefinerVisitor<'tcx> {
    fn visit_terminator(&mut self, terminator: &Terminator<'tcx>, _location: Location) {
        match &terminator.kind {
            TerminatorKind::Call { func, args, .. } => {
                self.refine_rec(func, args);
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
