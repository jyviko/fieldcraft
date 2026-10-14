# Block sliding to a stop on a deformable plate (explicit contact, decaying friction)

The block of `explicit-sliding-block-wall` (1 × 1 × 0.5, 4 × 4 × 2 hex8, E = 10⁵, ν = 0.3,
ρ = 1) rests on a plate 4.5 × 2 × 0.5 of 9 × 4 × 1 hex8 held at its base, the two meshes not
matching; a contact pair joins the plate's top and the block's bottom with Coulomb friction whose
coefficient falls with the slip rate, μ = 0.2 + 0.3 e^(−1.5 v). Under gravity g = 10 the block
is launched along x at v₀ = 3. A rigid block decelerates at g μ(v), stops after 1.2001 and travels
∫₀^v₀ v dv / (g μ(v)) = 2.01524. Solved to t = 1.56.
