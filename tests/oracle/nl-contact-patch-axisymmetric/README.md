# Contact patch test, axisymmetric (implicit mortar contact, non-matching edges)

The axisymmetric form of `nl-contact-patch`: two rings of unit square section, 1 ≤ r ≤ 2, stacked
along the axis y and meshed independently (3 × 2 quad4 below, graded in r; 2 × 2 above), so the
nodes of the interface at y = 1 do not match. A frictionless contact pair joins the lower ring's
top edge (main) and the upper ring's bottom edge (secondary). The base y = 0 is held in y and
the top y = 2 pushed down by 0.02 (E = 1000, ν = 0.3), in a `Small` (small sliding) and a `Finite`
(finite sliding) nonlinear static step. Axial compression with free radial expansion is a
uniform uniaxial stress σ_yy in both rings (u_r = −ν ε r, so the hoop strain equals the radial
one), carried by a uniform contact pressure; the edge mortar integrals weight by 2πr, so the
patch test checks that the radius enters the gaps and the forces consistently.
