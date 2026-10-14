# Neo-Hookean cube: explicit against implicit (quasi-static)

A unit cube of 2 × 2 × 2 cells of tet4 (six per cell) of a compressible neo-Hookean material
(E = 10, ν = 0.3, ρ = 1), quarter symmetric (symmetry planes x = 0, y = 0, z = 0), pulled on
x = 1 to λ = 1.5 in uniaxial stress. The same model is solved by the implicit nonlinear solver
and by explicit dynamics with the pull applied over 100 time units (a few hundred wave transits):
the lateral contraction and the axial stress must agree, both coming from the shared
finite-strain material point.
