use crate::paths::node::Path;

pub struct PathRanker;

impl PathRanker {
    /// Port of reference Pathfinder::rankPaths
    pub fn rank_paths(paths: &mut Vec<Path>) {
        // Sort by quality (descending) then liquidity (descending)
        paths.sort_by(|a, b| {
            b.quality
                .partial_cmp(&a.quality)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.liquidity
                        .partial_cmp(&a.liquidity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
    }

    pub fn is_valid(path: &Path) -> bool {
        // Implement path validation rules (e.g., no cycles, valid transitions)
        !path.nodes.is_empty()
    }
}
