use std::collections::hash_map::Entry;

use itertools::Itertools;
use petgraph::prelude::DiGraphMap;

use crate::*;

/// Compute the cost of a given item in terms of another one.
pub trait CostIn<O> {
    const COST: u32;
}
impl<O, T> CostIn<O> for T {
    default const COST: u32 = <T as InputCost<O>>::COST + <T as SelfCost<O>>::COST;
}
impl<O> CostIn<O> for () {
    const COST: u32 = 0;
}
impl<O, A: CostIn<O>> CostIn<O> for (A,) {
    const COST: u32 = A::COST;
}
impl<O, A: CostIn<O>, B: CostIn<O>> CostIn<O> for (A, B) {
    const COST: u32 = A::COST + B::COST;
}
impl<O, A: CostIn<O>, B: CostIn<O>, C: CostIn<O>> CostIn<O> for (A, B, C) {
    const COST: u32 = A::COST + B::COST + C::COST;
}
impl<O, T: BaseRecipe> CostIn<O> for T {
    const COST: u32 = 0;
}
impl<O, R: CostIn<O>, const N: usize> CostIn<O> for [R; N] {
    const COST: u32 = N as u32 * R::COST;
}
impl<O, R: CostIn<O> + BundleMakeable, const N: u32> CostIn<O> for Bundle<R, N> {
    const COST: u32 = N * R::COST;
}

trait SelfCost<A> {
    const COST: u32;
}
impl<A, B> SelfCost<A> for B {
    default const COST: u32 = 0;
}
impl<A> SelfCost<A> for A {
    const COST: u32 = 1;
}
// impl<A: ResourceType, const N: u32> SelfCost<A> for Bundle<A, N> {
//     const COST: u32 = N;
// }

trait InputCost<A> {
    const COST: u32;
}
impl<O, T> InputCost<O> for T {
    default const COST: u32 = 0;
}
impl<O, T> InputCost<O> for T
where
    Self: SingleMakeable<Input: CostIn<O>>,
{
    const COST: u32 = <<Self as SingleMakeable>::Input as CostIn<O>>::COST;
}

const _: () = {
    assert!(<IronOre as CostIn<IronOre>>::COST == 1);
    // assert!(<Bundle<Iron, 1> as InputCost<IronOre>>::COST == 1);
    // assert!(<Miner as CostIn<IronOre>>::COST == 10);
    // assert!(<Miner as CostIn<Bundle<IronOre, 1>>>::COST == 10);
    // assert!(<Miner as CostIn<CopperOre>>::COST == 5);
    // assert!(<Furnace<IronSmelting> as CostIn<IronOre>>::COST == 10);
    // assert!(<Iron as CostIn<Iron>>::COST == 1);
};

/// Resource dependency graph.
#[derive(Default)]
pub struct ResourceGraph {
    name_map: HashMap<GraphNode, String>,
    graph: DiGraphMap<GraphNode, f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphNode(TypeId);

impl std::fmt::Display for ResourceGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first_col: Vec<_> = vec![];
        let mut rows: Vec<Vec<_>> = vec![];
        for id in self.graph.nodes() {
            first_col.push(self.name_map.get(&id).unwrap());
            rows.push(
                self.graph
                    .neighbors(id)
                    .map(|tgt| {
                        let w = self.graph.edge_weight(id, tgt).unwrap();
                        let name = self.name_map.get(&tgt).unwrap();
                        format!("{w} {name}")
                    })
                    .collect_vec(),
            );
        }

        let first_col_width = first_col.iter().map(|s| s.len()).max().unwrap();
        let other_cols_width = rows
            .iter()
            .flat_map(|col| col.iter())
            .map(|s| s.len())
            .max()
            .unwrap();

        for (first, row) in first_col.into_iter().zip(rows) {
            let row = row
                .into_iter()
                .map(|x| format!("{x:w$}", w = other_cols_width))
                .format(" ");
            writeln!(f, "{first:w$}  takes:  {}", row, w = first_col_width)?;
        }
        Ok(())
    }
}

impl ResourceGraph {
    fn node_for<T: Any>() -> GraphNode {
        GraphNode(TypeId::of::<T>())
    }

    /// Add the node to the graph, and returns its id if that was the first time we added that
    /// node.
    pub fn add_node<T: Any>(&mut self) -> Option<GraphNode> {
        let node = Self::node_for::<T>();
        match self.name_map.entry(node) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                entry.insert(type_name::<T>());
                Some(node)
            }
        }
    }

    /// Add the node to the graph, and returns its id if that was the first time we added that
    /// node.
    pub fn add_edge_to<T: Any>(&mut self, start: GraphNode, weight: f32) {
        let to = Self::node_for::<T>();
        self.graph.add_edge(start, to, weight);
    }
}
