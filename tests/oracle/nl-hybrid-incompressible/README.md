# Nearly incompressible hybrid block under prescribed stretch

A unit cube of 3 × 3 × 3 hybrid hex8 elements on three symmetry planes, neo-Hookean with E = 3,
ν = 0.4999 (κ/μ ≈ 5000), pulled to λ = 2 by a prescribed displacement in a nonlinear static
step. The first iterations of an increment strain only the elements next to the moving face,
whose mixed tangents are then indefinite: the step relies on the LDLᵀ factorization (before it,
every increment was cut back until the step hit its cap). The reference is the compressible
decoupled energy's uniaxial stress with the lateral stretch found by bisection so σ_yy = 0.
