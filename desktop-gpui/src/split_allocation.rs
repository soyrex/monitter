//! Proportional allocation for inserting a physical pane into one dock region.
//! Tab groups are leaves; existing tabs do not consume additional pane area.
use gpui_kit::component::dock::{DockSplitSnapshot, NodeId, PaneNode, PaneRef};
use std::collections::HashMap;

pub(super) struct SplitAllocation {
    pub(super) placement: gpui_kit::component::dock::DockPlacement,
    weights: HashMap<NodeId, f32>,
    new_weight: f32,
}

fn normalized(values: &[f32]) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }
    let total: f32 = values.iter().sum();
    if total.is_finite() && total > 0. && values.iter().all(|v| v.is_finite() && *v > 0.) {
        values.iter().map(|value| value / total).collect()
    } else {
        vec![1. / values.len() as f32; values.len()]
    }
}

fn borrow_proportionally(values: &[f32]) -> Vec<f32> {
    if values.is_empty() {
        return vec![1.];
    }
    let factor = values.len() as f32 / (values.len() + 1) as f32;
    let mut result = normalized(values)
        .into_iter()
        .map(|weight| weight * factor)
        .collect::<Vec<_>>();
    result.push(1. / (values.len() + 1) as f32);
    result
}

pub(super) fn leaf_weights(root: &PaneNode, splits: &[DockSplitSnapshot]) -> HashMap<NodeId, f32> {
    fn walk(
        node: &PaneNode,
        weight: f32,
        splits: &[DockSplitSnapshot],
        result: &mut HashMap<NodeId, f32>,
    ) {
        match node.kind() {
            PaneRef::Tabs { panels, .. } if !panels.is_empty() => {
                result.insert(node.id(), weight);
            }
            PaneRef::Split {
                children, sizes, ..
            } => {
                let shares = splits
                    .iter()
                    .find(|split| split.node == node.id())
                    .map(|split| normalized(&split.shares))
                    .unwrap_or_else(|| {
                        let given: Vec<_> = sizes
                            .iter()
                            .filter_map(|size| size.map(|size| size.as_f32()))
                            .filter(|size| *size > 0. && size.is_finite())
                            .collect();
                        let fallback = if given.is_empty() {
                            1.
                        } else {
                            given.iter().sum::<f32>() / given.len() as f32
                        };
                        normalized(
                            &(0..children.len())
                                .map(|ix| {
                                    sizes
                                        .get(ix)
                                        .copied()
                                        .flatten()
                                        .map(|size| size.as_f32())
                                        .unwrap_or(fallback)
                                })
                                .collect::<Vec<_>>(),
                        )
                    });
                for (child, share) in children.iter().zip(shares) {
                    walk(child, weight * share, splits, result);
                }
            }
            _ => {}
        }
    }
    let mut result = HashMap::new();
    walk(root, 1., splits, &mut result);
    result
}

impl SplitAllocation {
    pub(super) fn capture(
        placement: gpui_kit::component::dock::DockPlacement,
        root: &PaneNode,
        splits: &[DockSplitSnapshot],
    ) -> Self {
        let mut weights = leaf_weights(root, splits);
        let count = weights.len();
        if count > 0 {
            let total = weights.values().sum::<f32>();
            for weight in weights.values_mut() {
                *weight /= total;
            }
        }
        let nodes = weights.keys().copied().collect::<Vec<_>>();
        let allocated =
            borrow_proportionally(&nodes.iter().map(|node| weights[node]).collect::<Vec<_>>());
        for (node, weight) in nodes.into_iter().zip(allocated.iter().copied()) {
            weights.insert(node, weight);
        }
        Self {
            placement,
            weights,
            new_weight: *allocated.last().unwrap(),
        }
    }

    /// Set targets on the post-insertion native snapshots, retaining topology
    /// and all entities. Only ancestors affected by the extra leaf resize.
    pub(super) fn targets(
        &self,
        root: &PaneNode,
        new_leaf: NodeId,
        snapshots: &mut [DockSplitSnapshot],
    ) -> bool {
        if self.weights.is_empty() {
            return true;
        }
        let new_weight = self.new_weight;
        fn sum(
            node: &PaneNode,
            prior: &HashMap<NodeId, f32>,
            new_leaf: NodeId,
            new_weight: f32,
            snapshots: &mut [DockSplitSnapshot],
        ) -> Option<f32> {
            match node.kind() {
                PaneRef::Tabs { panels, .. } => {
                    if node.id() == new_leaf {
                        Some(new_weight)
                    } else if panels.is_empty() {
                        Some(0.)
                    } else {
                        prior.get(&node.id()).copied()
                    }
                }
                PaneRef::Split { children, .. } => {
                    let weights = children
                        .iter()
                        .map(|child| sum(child, prior, new_leaf, new_weight, snapshots))
                        .collect::<Option<Vec<_>>>()?;
                    let total = weights.iter().sum::<f32>();
                    if total <= 0. || !total.is_finite() {
                        return None;
                    }
                    // Center regions retain a canonical split root even when
                    // it has one child; it has no divider/snapshot to resize.
                    if children.len() == 1 {
                        return Some(total);
                    }
                    let split = snapshots.iter_mut().find(|split| split.node == node.id())?;
                    split.target_shares = weights.iter().map(|weight| weight / total).collect();
                    Some(total)
                }
            }
        }
        sum(root, &self.weights, new_leaf, new_weight, snapshots).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::borrow_proportionally;
    #[test]
    fn split_allocation_preserves_custom_ratios_and_balances_equal_siblings() {
        let allocated = borrow_proportionally(&[2., 3., 5.]);
        assert!((allocated[0] / allocated[1] - 2. / 3.).abs() < 0.00001);
        assert!((allocated[1] / allocated[2] - 3. / 5.).abs() < 0.00001);
        assert_eq!(borrow_proportionally(&[1., 1., 1.]), vec![0.25; 4]);
        assert!((allocated.iter().sum::<f32>() - 1.).abs() < 0.00001);
    }
}
