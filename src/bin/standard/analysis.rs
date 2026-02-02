use std::collections::hash_map::Entry;

use itertools::Itertools;
use petgraph::{
    matrix_graph::Zero,
    prelude::DiGraphMap,
    visit::{DfsPostOrder, Walker},
};

use crate::*;

/// Resource dependency graph.
#[derive(Default)]
pub struct ResourceGraph {
    name_map: HashMap<GraphNode, String>,
    graph: DiGraphMap<GraphNode, f32>,
    /// Where to start the DFS when displaying the graph.
    graph_root: Option<GraphNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphNode(TypeId);

impl std::fmt::Display for ResourceGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first_col: Vec<_> = vec![];
        let mut rows: Vec<Vec<_>> = vec![];
        let first_node = self
            .graph_root
            .unwrap_or_else(|| self.graph.nodes().next().unwrap());
        let topo_sort = DfsPostOrder::new(&self.graph, first_node)
            .iter(&self.graph)
            .collect_vec();
        for id in topo_sort.into_iter().rev() {
            // for id in self.graph.nodes() {
            first_col.push(self.name_map.get(&id).unwrap());
            rows.push(
                self.graph
                    .neighbors(id)
                    .map(|tgt| {
                        let w = self.graph.edge_weight(id, tgt).unwrap();
                        let name = self.name_map.get(&tgt).unwrap();
                        let w = if w.is_zero() {
                            String::new()
                        } else {
                            format!("{w} ")
                        };
                        format!("{w}{name}")
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

    /// Set the node to use as root when displaying the graph.
    pub fn set_display_root<T: Any>(&mut self) {
        self.graph_root = Some(Self::node_for::<T>())
    }
}
