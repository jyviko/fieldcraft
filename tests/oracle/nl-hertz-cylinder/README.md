# Hertz line contact: cylinder on a flat (implicit mortar contact)

A steel cylinder of radius R = 10 mm pressed on a steel block (E = 210 GPa, ν = 0.3; mm, N, MPa)
in plane strain: a quarter model of the cylinder's lower part (a 2.5 mm wide block whose bottom
is the circular arc, graded towards the contact) on a 5 × 5 mm foundation, symmetric about
x = 0, one element thick with every node held in z. A pressure on the indenter's top carries
the load P = 2265.57 N/mm (per length of the whole cylinder) chosen so that Hertz's half-width is
b = 0.5 mm and the peak pressure p₀ = 2884.6 MPa. Soft springs (0.01 N/mm per node) hold the
indenter's top vertically until the contact closes. One nonlinear static step without nonlinear
geometry: small sliding, mortar contact of the round face (secondary) on the flat (main) by an
augmented Lagrangian.
