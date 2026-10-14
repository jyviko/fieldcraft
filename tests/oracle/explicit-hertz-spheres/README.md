# Two elastic spheres colliding head-on (explicit contact, Hertz)

Two identical steel spheres, R = 10 mm, E = 210 GPa, ν = 0.3, ρ = 7.85 t/m³ (mm, t, s, N, MPa),
meet head-on at a relative speed of 1 m/s. Hertz's quasi-static theory gives the approach
δ = 0.020441 mm, the contact radius a = √(R* δ) = 0.320 mm, the peak force 1005.4 N and the
contact duration 60.16 μs. The case waits for a graded sphere mesh built from commands: the contact
radius is a thirtieth of the sphere's, so the mesh must be fine at the poles and coarse elsewhere.
The explicit crate's tests check the same theory on a cubed-sphere mesh generated in code.
