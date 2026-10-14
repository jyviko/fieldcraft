# Uniaxial tension of a J2 bilinear-hardening cube to large strain (implicit)

The cube of `j2-uniaxial-explicit` solved by the implicit nonlinear solver: one fully integrated
hex8, E = 200000, ν = 0.3, σ_y = 250, H = 2000 (isotropic), held by symmetry on three faces and
stretched on the fourth to λ = e^0.2 in a nonlinear static step, lateral faces free.

Simo's exponential return map integrates this proportional path exactly in the logarithmic
strain, so the Kirchhoff stress is the small-strain curve τ = σ_y + E_t (ε − σ_y/E) = 643.564 at
ε = 0.2 and the equivalent plastic strain (τ − σ_y)/H = 0.196782. The Cauchy stress is
σ = τ/J with Hencky's volume law ln J = τ/(3K): 642.737.

Cross-check with the explicit solver: `j2-uniaxial-explicit` reaches 643.57 for the Cauchy stress
with the same plastic strain. Its hypoelastic update takes the logarithmic strain rate against
the Cauchy stress, while the implicit model is hyperelastic in the Kirchhoff stress: the two
laws differ by the factor J = 1.0013 (the elastic volume change), which is the 0.13 % between
them; both meet the closed form of their own law.
