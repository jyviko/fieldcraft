# Contact patch test (explicit contact, non-matching meshes)

Two unit cubes stacked along z, meshed independently: 3 × 3 × 2 hex8 below, 2 × 2 × 2 above, so
the nodes of the interface at z = 1 do not match. A frictionless contact pair joins the lower
block's top and the upper block's bottom. The faces x = 0 and y = 0 of both blocks are symmetry
planes (normal displacement held), the base z = 0 is held in z, and the top z = 2 is pushed down by
0.01 at constant speed over a hundred wave transits (E = 1000, ν = 0.3, ρ = 1). The exact state
is uniaxial stress, uniform in both blocks; the stresses are compared as ratios, since the penalty
takes a small, uniform share of the squeeze.
