# Tie patch test (non-matching meshes)

Two unit cubes stacked along z, meshed independently: 3 × 3 × 2 hex8 below (graded in x and y),
2 × 2 × 2 above, so the nodes at z = 1 do not match. A tie bonds the upper block's bottom
(secondary) to the lower block's top (main). The faces x = 0 and y = 0 are symmetry planes, the
base z = 0 is held in z, and the top z = 2 is pulled up by 0.02 (E = 1000, ν = 0.3) in a linear
static step. The exact state is the uniaxial stress σ_zz = 10, uniform in both blocks.
