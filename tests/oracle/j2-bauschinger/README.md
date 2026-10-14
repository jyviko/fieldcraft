# Bauschinger effect of linear kinematic hardening

One hex8 cube (E = 200000, ν = 0.3, σ_y = 250, H = 2000, Prager's linear kinematic hardening)
held by symmetry on three faces, its fourth face pulled to a strain of 0.01 and brought back to
zero within one nonlinear static step without nonlinear geometry, by the load path
[[0, 0], [0.5, 1], [1, 0]] in increments of 0.05.

At ε_a the stress is σ_a = σ_y + E_t (ε_a − σ_y/E) = 267.327. The yield surface keeps its size
and moves with the back stress, so unloading is elastic only until σ_a − 2σ_y and then hardens
with E_t: back at zero strain σ = σ_a − 2σ_y − E_t (ε_a − 2σ_y/E) = −247.525. Isotropic hardening
would yield again only at −σ_a and end at −281.835. The equivalent plastic strain accumulates
(σ_a − σ_y)/H forward and E/(E + H)(ε_a − 2σ_y/E) in reverse, 0.0160891.
