# Constant-stress patch test, hex20

A unit cube of eight hex20 cells whose shared points are moved off the regular grid (every
face, edge and centre point; the corners stay), so every element is distorted (one mesh.block per cell). Symmetry on the
faces x = 0, y = 0 and z = 0; tractions 1, 2 and 3 on the faces x = 1, y = 1 and z = 1
(E = 1000, ν = 0.25). The exact solution is the uniform stress diag(1, 2, 3) with strains
ε_i = (σ_i − ν(σ_j + σ_k))/E, which every conforming element of a convergent formulation
reproduces exactly on any mesh.
