# Contact patch test in plane strain (implicit mortar contact, non-matching edges)

The 2D form of `nl-contact-patch`: two unit squares stacked along y in plane strain (2 mm thick),
meshed independently: 3 × 2 quad4 below (graded in x), 2 × 2 above, so the nodes of the interface
at y = 1 do not match. A frictionless contact pair joins the lower block's top edge (main) and
the upper block's bottom edge (secondary). The edge x = 0 is a symmetry line, the base y = 0 is
held in y, and the top y = 2 is pushed down by 0.02 (E = 1000, ν = 0.3) in two nonlinear static
steps: `Small` without nonlinear geometry (small sliding) and `Finite` with it (finite sliding).
The exact state is uniaxial stress σ_yy (σ_zz = ν σ_yy out of plane), uniform in both blocks,
with a uniform contact pressure equal to it.
