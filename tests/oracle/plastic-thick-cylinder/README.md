# Elastic–perfectly plastic thick cylinder: plastic zone and collapse pressure

The Lamé cylinder of `lame-cylinder-axisymmetric` (a = 10, b = 20, an axisymmetric slice held
axially on both faces, so plane strain), now elastic–perfectly plastic: E = 200000,
ν = 0.4999 (Hill's solution takes the elasticity incompressible), σ_y = 250, one row of 20
reduced quad8 elements through the wall, in two nonlinear static steps without nonlinear
geometry:

- **Pressure**: the bore pressure p = k (2 ln(c/a) + 1 − c²/b²) = 180.195 (k = σ_y/√3) that
  takes the plastic zone to c = 15. The bore then moves u_a = k c²/(2 G a) = 0.024355: the plastic
  zone moves incompressibly (u r constant) and the elastic ring outside yields at its inner edge.
- **Collapse**: the bore pushed out by 0.3 (24 times its first-yield displacement), past the
  pressure at which the wall is plastic through; the radial force on the bore stays at the
  collapse pressure 2k ln(b/a) = 200.094 times 2πa per unit height.
