# Rotating disc with a central hole, plane stress

A thin disc, inner radius 20, outer 100 (E = 210000, ν = 0.3, ρ = 7.85e-9: mm, t, s, MPa),
spinning at 1000 rad/s about z: a quarter, one quad8 patch (16 × 32, graded 4 : 1 towards the
bore, where the hoop stress is steepest) with mid-side nodes on the arcs, symmetry on x = 0 and y = 0, a centrifugal load. Timoshenko and Goodier:
σ_r = (3 + ν)/8 ρω² (b² + a² − a²b²/r² − r²),
σ_θ = (3 + ν)/8 ρω² (b² + a² + a²b²/r² − (1 + 3ν)/(3 + ν) r²).
