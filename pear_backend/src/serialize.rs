use rustc_hash::{FxHashMap, FxHashSet};
use rustc_hir::def_id::DefId;
use rustc_middle::{
    mir::mono::MonoItem,
    ty::{FnSig, Instance},
};
use rustc_span::Span;
use serde::Serializer;

use crate::{reachability::Node, refiner::RefinedNode, TransitiveRefinedNode};

pub fn serialize_def_id<S>(def_id: &DefId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(format!("{def_id:?}").as_str())
}

pub fn serialize_mono_item<S>(mono_item: &MonoItem, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(mono_item.to_string().as_str())
}

pub fn serialize_edges<'tcx, S>(
    edges: &FxHashMap<MonoItem<'tcx>, FxHashSet<Node<'tcx>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_map(edges.iter().map(|(k, v)| (k.to_string(), v)))
}

pub fn serialize_refined_edges<'tcx, S>(
    edges: &FxHashMap<Instance<'tcx>, FxHashSet<RefinedNode<'tcx>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_map(edges.iter().map(|(k, v)| (k.to_string(), v)))
}

pub fn serialize_transitive_refined_edges<'tcx, S>(
    edges: &FxHashMap<Instance<'tcx>, FxHashSet<TransitiveRefinedNode<'tcx>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_map(edges.iter().map(|(k, v)| (k.to_string(), v)))
}

pub fn serialize_instance<S>(instance: &Instance, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(instance.to_string().as_str())
}

pub fn serialize_instance_vec<S>(
    instances: &Vec<Instance>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_seq(instances.iter().map(|instance| instance.to_string()))
}

pub fn serialize_span<S>(span: &Span, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(format!("{span:?}").as_str())
}

pub fn serialize_sig<S>(sig: &FnSig, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(sig.to_string().as_str())
}
