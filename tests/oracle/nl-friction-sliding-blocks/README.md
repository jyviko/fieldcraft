# Coulomb friction: a block slid on another (implicit mortar contact)

A unit cube (2 × 2 × 2 hex8) on a 2 × 1 × 1 block (6 × 3 × 2 hex8), meshes not matching, E = 1000,
ν = 0.3. The base of the lower block is held; the top of the upper block is pressed down by 0.02
and moved 0.1 sideways in proportion over one nonlinear static step without nonlinear geometry.
The contact pair has μ_s = 0.3 and μ_k = 0.2: the sideways demand far exceeds the friction, so
the interface slips, with the kinetic coefficient once the nodes slip, and carries μ_k N.
