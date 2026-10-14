# Hertz line contact in plane strain (implicit mortar contact of 2D edges)

The 2D form of `nl-hertz-cylinder`: the same steel cylinder of radius R = 10 mm on a steel block
(E = 210 GPa, ν = 0.3; mm, N, MPa), now a plane strain model of unit thickness meshed with quad4:
the cylinder's lower part a 2.5 mm wide block whose bottom edge is the circular arc (20 × 10,
graded towards the contact), on a 5 × 5 mm foundation (24 × 14), symmetric about x = 0. A
pressure on the indenter's top edge carries the load P = 2265.57 N/mm (per length of the whole
cylinder) for Hertz's half-width b = 0.5 mm and peak pressure p₀ = 2884.6 MPa. Soft springs hold
the indenter's top until the contact closes. One nonlinear static step without nonlinear
geometry: small sliding, edge-to-edge mortar contact of the round edge (secondary) on the flat
(main) by an augmented Lagrangian. The references and tolerances are the 3D case's, so the two
discretisations of one problem are checked against one closed form.
