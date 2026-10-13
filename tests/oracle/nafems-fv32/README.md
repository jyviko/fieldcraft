# NAFEMS FV32 cantilevered tapered membrane, free vibration (hex20 plane stress layer)

Our model of the published problem: a membrane 10 m long tapering symmetrically from 5 m at the
clamped edge to 1 m at the free end, 0.05 m thick, E = 200 GPa, ν = 0.3, ρ = 8000 kg/m³;
u_x = u_y = 0 on the root. In-plane (plane stress) vibration is modelled by half the thickness of
hex20 elements (16 × 8 × 1) with u_z = 0 on the mid-surface and the other face free.
