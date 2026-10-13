# NAFEMS LE1 elliptic membrane (hex20 plane stress layer)

Our model of the published problem: the quarter membrane between the ellipses (x/2)² + y² = 1 and
(x/3.25)² + (y/2.75)² = 1, thickness 0.1, E = 210 GPa, ν = 0.3 (MPa and m), u_x = 0 on AB (x = 0),
u_y = 0 on CD (y = 0), an outward pressure of 10 MPa on the outer edge BC. Plane stress is
modelled by half the thickness of hex20 elements with a symmetry plane at the mid-surface and the
other face free: 24 × 48 elements graded 4 : 1 towards the hole, converged at D (32 × 64 with
the same grading changes σ_yy there by 0.003%).
