use crate::{CardinalDirection, CellId, SphereError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoverageRole {
    Core,
    Transition,
    Halo,
    Ambient,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PatchRequest {
    pub patch_id: String,
    pub core_parents: Vec<CellId>,
    pub target_level: u8,
    pub halo_parent_rings: u8,
    pub cell_budget: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregationWeight {
    pub child: CellId,
    pub true_area_weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatchPlan {
    pub patch_id: String,
    pub base_level: u8,
    pub target_level: u8,
    pub core: BTreeSet<CellId>,
    pub transition_leaves: BTreeSet<CellId>,
    pub halo: BTreeSet<CellId>,
    pub core_parents: BTreeSet<CellId>,
}

impl PatchPlan {
    /// Builds a deterministic L→L+2 patch with an L+1 transition ring.
    ///
    /// # Errors
    /// Rejects mixed parent levels, unsupported level gaps, empty cores and
    /// requests exceeding the declared cell budget.
    pub fn build(request: PatchRequest) -> Result<Self, SphereError> {
        let core_parents: BTreeSet<_> = request.core_parents.into_iter().collect();
        let Some(first) = core_parents.first().copied() else {
            return Err(SphereError::InvalidPatch("core is empty".into()));
        };
        let base_level = first.level();
        if core_parents.iter().any(|cell| cell.level() != base_level) {
            return Err(SphereError::InvalidPatch(
                "core parents use mixed levels".into(),
            ));
        }
        if request.target_level != base_level + 2 {
            return Err(SphereError::InvalidPatch(
                "E1 requires target level = base level + 2".into(),
            ));
        }

        let transition_parents = neighbour_ring(&core_parents, 1)?;
        let mut support_parents = core_parents.clone();
        let mut frontier = core_parents.clone();
        for _ in 0..request.halo_parent_rings.max(1) {
            let ring = neighbour_ring(&frontier, 1)?;
            support_parents.extend(ring.iter().copied());
            frontier = ring;
        }

        let mut core = BTreeSet::new();
        for parent in &core_parents {
            core.extend(descendants(*parent, request.target_level)?);
        }
        let mut transition_leaves = BTreeSet::new();
        for parent in &transition_parents {
            transition_leaves.extend(parent.children()?);
        }
        let mut halo = BTreeSet::new();
        for parent in support_parents.difference(&core_parents) {
            halo.extend(descendants(*parent, request.target_level)?);
        }

        let required = core.len() + transition_leaves.len() + halo.len();
        if required > request.cell_budget {
            return Err(SphereError::PatchBudgetExceeded {
                required,
                budget: request.cell_budget,
            });
        }
        Ok(Self {
            patch_id: request.patch_id,
            base_level,
            target_level: request.target_level,
            core,
            transition_leaves,
            halo,
            core_parents,
        })
    }

    #[must_use]
    pub fn coverage_role(&self, cell: CellId) -> CoverageRole {
        if self.core.contains(&cell) {
            CoverageRole::Core
        } else if self.transition_leaves.contains(&cell) {
            CoverageRole::Transition
        } else if self.halo.contains(&cell) {
            CoverageRole::Halo
        } else {
            CoverageRole::Ambient
        }
    }

    /// Returns true-area child weights for a core parent.
    ///
    /// # Errors
    /// Rejects a parent outside the patch core.
    pub fn aggregation_weights(
        &self,
        parent: CellId,
    ) -> Result<Vec<AggregationWeight>, SphereError> {
        if !self.core_parents.contains(&parent) {
            return Err(SphereError::InvalidPatch(
                "parent is outside patch core".into(),
            ));
        }
        let children = descendants(parent, self.target_level)?;
        let parent_area = parent.unit_sphere_area();
        Ok(children
            .into_iter()
            .map(|child| AggregationWeight {
                child,
                true_area_weight: child.unit_sphere_area() / parent_area,
            })
            .collect())
    }

    #[must_use]
    pub fn discrete_signature(&self) -> String {
        let list = |cells: &BTreeSet<CellId>| {
            cells
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{}|{}|{}|{}",
            self.patch_id,
            list(&self.core),
            list(&self.transition_leaves),
            list(&self.halo)
        )
    }
}

fn neighbour_ring(cells: &BTreeSet<CellId>, width: u8) -> Result<BTreeSet<CellId>, SphereError> {
    let mut reached = cells.clone();
    let mut frontier = cells.clone();
    for _ in 0..width {
        let mut next = BTreeSet::new();
        for cell in &frontier {
            for direction in CardinalDirection::ALL {
                next.insert(cell.neighbour(direction)?);
            }
        }
        next.retain(|cell| !reached.contains(cell));
        reached.extend(next.iter().copied());
        frontier = next;
    }
    reached.retain(|cell| !cells.contains(cell));
    Ok(reached)
}

fn descendants(root: CellId, level: u8) -> Result<Vec<CellId>, SphereError> {
    if level < root.level() {
        return Err(SphereError::InvalidLevel(level));
    }
    let shift = level - root.level();
    let width = 1_u32 << shift;
    let start_i = root.i() << shift;
    let start_j = root.j() << shift;
    let side = usize::try_from(width)
        .map_err(|_| SphereError::InvalidPatch("descendant count exceeds platform size".into()))?;
    let capacity = side
        .checked_mul(side)
        .ok_or_else(|| SphereError::InvalidPatch("descendant count overflows".into()))?;
    let mut result = Vec::with_capacity(capacity);
    for j in start_j..start_j + width {
        for i in start_i..start_i + width {
            result.push(CellId::new(root.face(), level, i, j)?);
        }
    }
    Ok(result)
}
