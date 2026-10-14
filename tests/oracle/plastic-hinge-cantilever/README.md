# Plastic collapse of a cantilever

A steel bar 200 long with a 10 × 10 section (E = 200000, ν = 0.3, elastic–perfectly plastic at
σ_y = 250), reduced hex20 20 × 2 × 4, in a nonlinear static step without nonlinear geometry. The
root x = 0 is a plane of symmetry (no rotation, the section free to contract laterally), held in
Y, Z and against twist on average through a distributing coupling; a coupling at the tip pushes
the end down by 15, 4.5 times the deflection at first yield.

Beam theory's collapse is a plastic hinge at the root: M_p = σ_y b h²/4 = 62500 and
P_c = M_p/L = 312.5. The solid's collapse lies above it, since a continuum cannot rotate about a
hinge (that needs a jump of the normal velocity across the section): refined to 8 elements
through the depth or 160 along the length, the force settles at 322.3, 3.1 % above. The case
checks that the implicit plasticity reaches this plateau and holds it.
