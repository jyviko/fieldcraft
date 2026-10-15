//! The commands' parameter schemas ([`crate::schema`]), area by area. Each describes the JSON
//! the command's `run` reads, as its `params` doc string does in words.

use crate::schema::Collection as C;
use crate::schema::Dim as D;
use crate::schema::*;

// ---- Shared pieces ----

/// `{"set": …}` or `{"reference_point": …}`.
const TARGET_OPTIONS: &[Field] = &[
    reference("set", "Set", C::Set).help("A named set of nodes, elements or faces."),
    reference("reference_point", "Reference point", C::ReferencePoint).help("A reference point: a single node that stands for a rigid connection."),
];
const fn target(key: &'static str, label: &'static str) -> Field {
    either(key, label, TARGET_OPTIONS)
}

/// The name of an existing definition, for delete commands.
macro_rules! existing {
    ($label:literal, $c:expr) => {
        &[reference("name", $label, $c).req().help(concat!("The ", $label, " to act on, by name."))]
    };
}

pub const SET_DELETE: &[Field] = existing!("Set", C::Set);
pub const MATERIAL_DELETE: &[Field] = existing!("Material", C::Material);
pub const SECTION_DELETE: &[Field] = existing!("Section", C::Section);
pub const REFERENCE_POINT_DELETE: &[Field] = existing!("Reference point", C::ReferencePoint);
pub const COUPLING_DELETE: &[Field] = existing!("Coupling", C::Coupling);
pub const MASS_DELETE: &[Field] = existing!("Point mass", C::Mass);
pub const SPRING_DELETE: &[Field] = existing!("Spring", C::Spring);
pub const BOUNDARY_CONDITION_DELETE: &[Field] = existing!("Boundary condition", C::BoundaryCondition);
pub const LOAD_DELETE: &[Field] = existing!("Load", C::Load);
pub const FLUID_DOMAIN_DELETE: &[Field] = existing!("Fluid domain", C::FluidDomain);
pub const CONTACT_DELETE: &[Field] = existing!("Contact", C::Contact);
pub const TIE_DELETE: &[Field] = existing!("Tie", C::Tie);
pub const STEP_DELETE: &[Field] = existing!("Step", C::Step);
pub const SET_RESOLVE: &[Field] = existing!("Set", C::Set);

// ---- Model definitions ----

pub const SET: &[Field] = &[
    NAME,
    rows(
        "geometry",
        "Geometry",
        &[
            reference("body", "Body", C::Body).help("A body of the open design.").req(),
            text("face", "Face").help("A persistent face name of the body (e.g. F3); the whole body when empty.").hint("persistent face name"),
            text("edge", "Edge").help("A persistent edge name of the body."),
            text("vertex", "Vertex").help("A persistent vertex name of the body."),
        ],
    )
    .help("Bodies of the design, or their named faces, edges or vertices. Kept by name, so the set survives remeshing.")
    .hint("whole bodies, or their faces, edges or vertices"),
    labels("nodes", "Nodes", LabelOf::Node).help("Mesh node labels; Use selection takes the picked nodes.").hint("labels: 1, 2, 10-20"),
    labels("elements", "Elements", LabelOf::Element)
        .help("Mesh element labels; Use selection takes the picked elements.")
        .hint("labels: 1, 2, 10-20"),
    json("faces", "Element faces").help("Element faces, as a JSON list of element label and local face number.").hint("[{\"element\", \"face\"}]"),
    json("edges", "Element edges").help("Element edges, as a JSON list of element label and local edge number.").hint("[{\"element\", \"edge\"}]"),
];

const ELASTIC: Field = variants(
    "elastic",
    "Elasticity",
    &[
        variant(
            "isotropic",
            "Isotropic",
            &[
                num("young", "Young's modulus", D::Stress)
                    .help("Stiffness in tension: stress over elastic strain. Steel ≈ 210 GPa (210000 MPa), aluminium ≈ 70 GPa, rubber a few MPa.")
                    .req(),
                range("poisson", "Poisson's ratio", D::None, -1.0, 0.5)
                    .help("Lateral contraction over axial stretch, between -1 and 0.5. Metals ≈ 0.3, rubber close to 0.5.")
                    .req(),
            ],
        ),
        variant(
            "engineering_constants",
            "Orthotropic",
            &[
                vector("young", "E1, E2, E3", D::Stress, &["E1", "E2", "E3"]).help("Young's moduli along the three material axes.").req(),
                vector("poisson", "ν12, ν13, ν23", D::None, &["ν12", "ν13", "ν23"])
                    .help("Poisson's ratios between the material axes (contraction along the second axis for a stretch along the first).")
                    .req(),
                vector("shear", "G12, G13, G23", D::Stress, &["G12", "G13", "G23"]).help("Shear moduli in the three material planes.").req(),
            ],
        ),
    ],
)
.help("How the material deforms elastically: the same in every direction (isotropic) or per material axis (orthotropic).")
.def(r#"{"type": "isotropic"}"#);

const HARDENING: &[(&str, &str)] = &[("isotropic", "Isotropic"), ("kinematic", "Kinematic")];

const PLASTIC: Field = variants(
    "plastic",
    "Plasticity",
    &[
        variant(
            "bilinear",
            "Bilinear",
            &[
                positive("yield_stress", "Yield stress", D::Stress).help("The initial yield stress: where elastic behaviour ends and plastic flow starts. 6061-T6 aluminium ≈ 276 MPa, mild steel ≈ 250 MPa.").req(),
                num("hardening_modulus", "Hardening modulus", D::Stress).help("Slope of stress against plastic strain after yield. 0 is perfectly plastic; metals are often 1–5 % of Young's modulus (e.g. 2000 MPa for steel).").def("0"),
                choice("hardening", "Hardening", HARDENING).help("Isotropic: the yield surface grows equally in every direction. Kinematic: it moves with the loading, so reversed loading yields earlier (Bauschinger effect; for cyclic loads).").def("\"isotropic\""),
            ],
        ),
        variant(
            "table",
            "Table",
            &[rows(
                "points",
                "Hardening curve",
                &[positive("strain", "Plastic strain", D::None).help("Equivalent plastic strain of the point; the first point is 0.").req(), positive("stress", "Stress", D::Stress).help("Yield stress at that plastic strain.").req()],
            ).help("Points of yield stress against plastic strain, plastic strain rising from 0 at the initial yield.")
            .req()],
        ),
        variant(
            "johnson_cook",
            "Johnson–Cook",
            &[
                positive("a", "A", D::Stress).help("Johnson–Cook initial yield stress at the reference strain rate and room temperature. Steel ≈ 300–800 MPa.").req(),
                num("b", "B", D::Stress).help("Johnson–Cook hardening coefficient: the stress added by plastic strain.").req(),
                num("n", "n", D::None).help("Johnson–Cook hardening exponent, typically 0.1–0.6.").req(),
                num("c", "C", D::None).help("Johnson–Cook strain-rate coefficient, typically 0.01–0.03; 0 ignores the strain rate.").def("0"),
                num("m", "m", D::None).help("Johnson–Cook thermal softening exponent, typically about 1.").def("1"),
                positive("reference_strain_rate", "Reference strain rate", D::None).help("The plastic strain rate at which A, B and n were measured, per time unit (often 1/s).").def("1").hint("per time unit"),
                num("room_temperature", "Room temperature", D::Temperature).help("The temperature at which A, B and n hold; no thermal softening below it.").req(),
                num("melt_temperature", "Melt temperature", D::Temperature).help("The temperature at which strength drops to zero. Steel ≈ 1800 K, aluminium ≈ 900 K.").req(),
                positive("specific_heat", "Specific heat", D::SpecificHeat).help("Heat that raises a unit mass by one degree, for heating by plastic work. Steel ≈ 4.6e8 mm²/(s²·K) in mm-t-s.").hint("isothermal when empty"),
                range("taylor_quinney", "Taylor–Quinney", D::None, 0.0, 1.0).help("Fraction of plastic work turned into heat, 0 to 1; usually 0.9.").def("0.9"),
            ],
        ),
    ],
).help("Permanent deformation once the stress passes the yield stress: bilinear, a table of points, or Johnson–Cook.");

const DAMAGE: Field = variants(
    "damage",
    "Damage",
    &[variant(
        "johnson_cook",
        "Johnson–Cook",
        &[
            num("d1", "D1", D::None).help("Johnson–Cook failure strain constant. Typical metals: −0.8 to 0.2.").req(),
            num("d2", "D2", D::None).help("Johnson–Cook failure strain constant multiplying the stress-triaxiality exponential.").req(),
            num("d3", "D3", D::None).help("Johnson–Cook stress-triaxiality exponent constant, usually negative.").req(),
            num("d4", "D4", D::None).help("Johnson–Cook strain-rate constant; 0 when empty."),
            num("d5", "D5", D::None).help("Johnson–Cook temperature constant; 0 when empty."),
        ],
    )],
)
.help("When the material fails: Johnson–Cook failure strain built from D1–D5.");
const EROSION: Field = group(
    "erosion",
    "Erosion",
    &[positive("plastic_strain", "Plastic strain", D::None)
        .help("Equivalent plastic strain at which an element is removed, e.g. 0.3–1 for ductile metals.")
        .req()],
)
.help("Removes an element from an explicit analysis once it reaches the plastic strain given.");

const HYPERELASTIC: Field = variants(
    "hyperelastic",
    "Hyperelasticity",
    &[
        variant("neo_hookean", "Neo-Hookean", &[]),
        variant(
            "mooney_rivlin",
            "Mooney–Rivlin",
            &[num("c01", "C01", D::Stress).help("Mooney–Rivlin second constant; C10 comes from the isotropic shear modulus.").req()],
        ),
        variant(
            "yeoh",
            "Yeoh",
            &[
                num("c20", "C20", D::Stress).help("Yeoh second-order constant.").req(),
                num("c30", "C30", D::Stress).help("Yeoh third-order constant.").req(),
            ],
        ),
    ],
)
.help("A large-strain rubber model (nonlinear steps), built on the isotropic constants.")
.hint("large strain in nonlinear steps, from the isotropic constants");

const THERMAL: Field = group(
    "thermal",
    "Thermal",
    &[
        json("conductivity", "Conductivity")
            .help("Thermal conductivity, one value or one per axis. Steel ≈ 50 W/(m·K), i.e. 50 in mm-t-s-N.")
            .hint("k or [k1, k2, k3]"),
        positive("specific_heat", "Specific heat", D::SpecificHeat)
            .help("Heat that raises a unit mass by one degree. Steel ≈ 460 J/(kg·K), 4.6e8 in mm-t-s."),
        json("expansion", "Expansion")
            .help("Thermal expansion coefficient, one value or one per axis, in 1/K. Steel ≈ 1.2e-5, aluminium ≈ 2.3e-5.")
            .hint("α or [α1, α2, α3], 1/K"),
        num("reference_temperature", "Reference temperature", D::Temperature).help("The temperature at which there is no thermal strain.").def("0"),
    ],
)
.help("Heat conduction, specific heat and thermal expansion, for heat-transfer and thermal-stress steps.");

pub const MATERIAL: &[Field] = &[
    NAME,
    ELASTIC,
    positive("density", "Density", D::Density)
        .help("Mass per volume; needed for dynamics, frequencies and gravity. Steel ≈ 7.85e-9 t/mm³ (7850 kg/m³), aluminium ≈ 2.7e-9 t/mm³."),
    PLASTIC,
    HYPERELASTIC,
    DAMAGE,
    EROSION,
    THERMAL,
    variants(
        "fluid",
        "Fluid",
        &[variant(
            "newtonian",
            "Newtonian",
            &[positive("viscosity", "Viscosity", D::Viscosity).help("Dynamic viscosity. Water ≈ 1e-3 Pa·s (1e-9 MPa·s), air ≈ 1.8e-5 Pa·s.").req()],
        )],
    )
    .help("Makes the material a fluid for a lattice Boltzmann flow analysis.")
    .hint("a fluid needs a density"),
];

pub const MATERIAL_PLASTICITY: &[Field] =
    &[reference("name", "Material", C::Material).help("The material to change.").req(), PLASTIC.req().def(r#"{"type": "bilinear"}"#)];
pub const MATERIAL_DAMAGE: &[Field] = &[
    reference("name", "Material", C::Material).help("The material to change.").req(),
    DAMAGE.def(r#"{"type": "johnson_cook"}"#).hint("give damage, erosion or both"),
    EROSION,
];

const PROFILE: Field = variants(
    "profile",
    "Profile",
    &[
        variant(
            "rectangle",
            "Rectangle",
            &[
                positive("width", "Width", D::Length).help("Cross-section width, along the section's first axis.").req(),
                positive("height", "Height", D::Length).help("Cross-section height, along the section's second axis.").req(),
            ],
        ),
        variant("circle", "Circle", &[positive("radius", "Radius", D::Length).help("Outer radius of the cross-section.").req()]),
        variant(
            "pipe",
            "Pipe",
            &[
                positive("radius", "Radius", D::Length).help("Outer radius of the pipe.").req(),
                positive("thickness", "Wall", D::Length).help("Wall thickness of the pipe.").req(),
            ],
        ),
        variant(
            "i",
            "I-beam",
            &[
                positive("height", "Height", D::Length).help("Overall height of the I-beam.").req(),
                positive("width", "Width", D::Length).help("Flange width.").req(),
                positive("web", "Web", D::Length).help("Web thickness.").req(),
                positive("flange", "Flange", D::Length).help("Flange thickness.").req(),
            ],
        ),
        variant(
            "box",
            "Box",
            &[
                positive("width", "Width", D::Length).help("Outer width of the box.").req(),
                positive("height", "Height", D::Length).help("Outer height of the box.").req(),
                positive("thickness", "Wall", D::Length).help("Wall thickness of the box.").req(),
            ],
        ),
    ],
)
.help("The beam's cross-section shape.")
.req()
.def(r#"{"type": "rectangle"}"#);

pub const SECTION: &[Field] = &[
    NAME,
    reference("set", "Region", C::Set).help("The set of elements the section applies to.").req(),
    reference("material", "Material", C::Material).help("The material of the section.").req(),
    variants(
        "kind",
        "Kind",
        &[
            variant("solid", "Solid", &[]),
            variant(
                "shell",
                "Shell",
                &[
                    positive("thickness", "Thickness", D::Length).help("Shell thickness.").req(),
                    range("offset", "Offset", D::None, -0.5, 0.5)
                        .help("Where the reference surface sits, as a fraction of the thickness: 0 mid-surface, ±0.5 top or bottom.")
                        .def("0")
                        .hint("fraction of the thickness"),
                    int("points", "Integration points", 1, 15)
                        .help("Points through the thickness. 5 is usual; more for plasticity through the thickness.")
                        .def("5"),
                ],
            ),
            variant(
                "beam",
                "Beam",
                &[
                    PROFILE,
                    xyz("orientation", "Orientation", D::None)
                        .help("A direction that sets the beam section's first axis (not along the beam).")
                        .req()
                        .def("[0, 0, 1]"),
                ],
            ),
            variant("rod", "Rod", &[positive("area", "Area", D::Area).help("Cross-section area of the rod.").req()]),
            variant(
                "plane_stress",
                "Plane stress",
                &[positive("thickness", "Thickness", D::Length).help("Out-of-plane thickness of the plane-stress elements.").req().def("1")],
            ),
            variant(
                "plane_strain",
                "Plane strain",
                &[positive("thickness", "Thickness", D::Length).help("Out-of-plane thickness of the plane-strain slice.").def("1")],
            ),
            variant("axisymmetric", "Axisymmetric", &[]),
        ],
    )
    .help("What the elements are: solids, shells, beams, rods or 2D elements.")
    .req()
    .def(r#"{"type": "solid"}"#),
    group(
        "formulation",
        "Formulation",
        &[
            choice("integration", "Integration", &[("full", "Full"), ("reduced", "Reduced")])
                .help("Full: every integration point. Reduced: fewer points, cheaper and less stiff, but may need hourglass control.")
                .def("\"full\""),
            flag("incompatible_modes", "Incompatible modes")
                .help("Extra bending modes for linear hexahedra, so a coarse hex8 mesh bends without locking.")
                .def("false"),
            flag("hybrid", "Hybrid").help("Pressure as an extra unknown, for nearly incompressible materials such as rubber.").def("false"),
        ],
    )
    .help("How the solid elements are integrated; the defaults suit most models."),
];

pub const REFERENCE_POINT: &[Field] = &[
    NAME,
    xyz("xyz", "Position", D::Length).help("The point's coordinates.").req().def("[0, 0, 0]"),
    int("label", "Node label", 0, u32::MAX as i64).help("The node label written to decks.").hint("0: the highest free label"),
];

const DOFS: Field = multi("dofs", "Degrees of freedom", &[("1", "UX"), ("2", "UY"), ("3", "UZ"), ("4", "URX"), ("5", "URY"), ("6", "URZ")])
    .help("Which of the six DOFs to couple (1–3 translations, 4–6 rotations); all when empty.")
    .hint("all");

pub const COUPLING: &[Field] = &[
    NAME,
    reference("reference_point", "Reference point", C::ReferencePoint).help("The reference point the region follows.").req(),
    reference("set", "Region", C::Set).help("The nodes or faces coupled to the reference point.").req(),
    choice("kind", "Kind", &[("kinematic", "Kinematic"), ("distributing", "Distributing")])
        .help("Kinematic: the region moves rigidly with the point. Distributing: forces spread over the region, which stays free to deform.")
        .req()
        .def("\"kinematic\""),
    DOFS,
];

pub const MASS: &[Field] = &[
    NAME,
    TARGET,
    positive("mass", "Mass", D::Mass).help("A lumped mass at each node of the target.").hint("per node; [mx, my, mz] in JSON"),
    vector("inertia", "Inertia", D::Inertia, &["I11", "I22", "I33", "I12", "I13", "I23"])
        .help("Rotary inertia about the reference point: three moments and three products.")
        .hint("about the node; needs a reference point"),
];

const SPRING_FIRST: Field = target("first", "First end").help("Where the spring starts: a set or a reference point.").req();
const SPRING_SECOND: Field = target("second", "Second end").help("Where the spring ends; ground when empty.").hint("ground when empty");
const SPRING_DIRECTION: Field = either(
    "direction",
    "Direction",
    &[
        int("dof", "DOF", 1, 6).help("1–3 translation along X, Y, Z; 4–6 rotation about them."),
        xyz("vector", "Vector", D::None).help("A direction vector; its length does not matter."),
        unit("line", "Along the ends").help("Along the line from the first end to the second."),
    ],
)
.help("The direction the spring acts in: one DOF, a vector, or the line between its ends.")
.req()
.def(r#"{"dof": 1}"#);

pub const SPRING: &[Field] = &[
    NAME,
    variants(
        "",
        "Type",
        &[
            variant(
                "spring",
                "Spring",
                &[
                    SPRING_FIRST,
                    SPRING_SECOND,
                    SPRING_DIRECTION,
                    positive("stiffness", "Stiffness", D::Stiffness).help("Force per unit stretch. For a 10 N/mm spring enter 10 in mm-t-s-N.").req(),
                ],
            ),
            variant(
                "dashpot",
                "Dashpot",
                &[
                    SPRING_FIRST,
                    SPRING_SECOND,
                    SPRING_DIRECTION,
                    positive("coefficient", "Coefficient", D::Damping).help("Damping force per unit velocity.").req(),
                ],
            ),
        ],
    )
    .help("What kind of connector: a spring (stiffness) or a dashpot (damping).")
    .req()
    .def("\"spring\""),
];

/// The target of a boundary condition, force, moment or temperature.
const TARGET: Field = target("target", "Target").help("Where it applies: a set or a reference point.").req();

pub const BOUNDARY_CONDITION: &[Field] = &[
    NAME,
    variants(
        "",
        "Type",
        &[
            variant("fixed", "Fixed", &[TARGET]),
            variant(
                "displacement",
                "Displacement",
                &[
                    TARGET,
                    free_vector("values", "Values", D::Length, &["u1", "u2", "u3", "ur1", "ur2", "ur3"])
                        .help("Displacements u1–u3 along X, Y, Z and rotations ur1–ur3; empty entries stay free.")
                        .req()
                        .def("[0, 0, 0, null, null, null]")
                        .hint("empty: free"),
                ],
            ),
            variant(
                "symmetry",
                "Symmetry",
                &[
                    TARGET,
                    choice("plane", "Plane normal", &[("x", "X"), ("y", "Y"), ("z", "Z")])
                        .help("The axis normal to the symmetry plane: its translation and the two rotations about the other axes are held.")
                        .req()
                        .def("\"x\""),
                ],
            ),
            variant("temperature", "Temperature", &[TARGET, num("value", "Temperature", D::Temperature).help("The prescribed temperature.").req()]),
        ],
    )
    .help("What is held: everything fixed, given displacements, a symmetry plane or a temperature.")
    .req()
    .def("\"fixed\""),
];

const DISTRIBUTION: Field = choice("distribution", "Distribution", &[("total", "Total"), ("per_node", "Per node")])
    .help("Total: the value is split over the target's nodes. Per node: every node gets the full value.")
    .def("\"total\"");
const LOAD_SET: Field = reference("set", "Region", C::Set).help("The faces or elements the load acts on.").req();

pub const LOAD: &[Field] = &[
    NAME,
    variants(
        "",
        "Type",
        &[
            variant(
                "force",
                "Force",
                &[TARGET, xyz("vector", "Force", D::Force).help("The force, X, Y and Z components in the global axes.").req(), DISTRIBUTION],
            ),
            variant(
                "moment",
                "Moment",
                &[TARGET, xyz("vector", "Moment", D::Moment).help("The moment, about the X, Y and Z axes.").req(), DISTRIBUTION],
            ),
            variant(
                "pressure",
                "Pressure",
                &[
                    LOAD_SET,
                    num("magnitude", "Pressure", D::Stress)
                        .help("Pressure normal to the faces; positive pushes into the surface. 1 MPa = 1 N/mm².")
                        .req()
                        .hint("positive pushes in"),
                ],
            ),
            variant(
                "traction",
                "Traction",
                &[LOAD_SET, xyz("vector", "Traction", D::Stress).help("Force per area on the faces, X, Y and Z components.").req()],
            ),
            variant(
                "gravity",
                "Gravity",
                &[
                    xyz("acceleration", "Acceleration", D::Acceleration)
                        .help("Gravity or other acceleration. Earth gravity downward is [0, 0, -9810] mm/s² in mm-t-s.")
                        .req(),
                    reference("set", "Region", C::Set).help("Where gravity acts; the whole model when empty.").hint("the whole model"),
                ],
            ),
            variant(
                "body_force",
                "Body force",
                &[LOAD_SET, xyz("vector", "Force per volume", D::ForcePerVolume).help("A body force per unit volume, X, Y and Z components.").req()],
            ),
            variant(
                "centrifugal",
                "Centrifugal",
                &[
                    num("angular_velocity", "Angular velocity", D::AngularVelocity)
                        .help("Spin speed in radians per time unit. 3000 rpm ≈ 314 rad/s.")
                        .req(),
                    xyz("point", "Point on axis", D::Length).help("A point on the axis of rotation.").req().def("[0, 0, 0]"),
                    xyz("axis", "Axis", D::None).help("Direction of the axis of rotation.").req().def("[0, 0, 1]"),
                    reference("set", "Region", C::Set).help("Where the centrifugal load acts; the whole model when empty.").hint("the whole model"),
                ],
            ),
            variant(
                "surface_flux",
                "Surface heat flux",
                &[LOAD_SET, num("flux", "Flux", D::HeatFlux).help("Heat flow per area into the surface.").req()],
            ),
            variant(
                "convection",
                "Convection",
                &[
                    LOAD_SET,
                    positive("film", "Film coefficient", D::FilmCoefficient)
                        .help("Convective heat transfer coefficient. Still air ≈ 5–25 W/(m²·K), water 500–10000.")
                        .req(),
                    num("sink", "Sink temperature", D::Temperature).help("The temperature of the surrounding fluid.").req(),
                ],
            ),
            variant(
                "heat_source",
                "Heat source",
                &[LOAD_SET, num("power", "Power per volume", D::None).help("Heat generated per unit volume.").req()],
            ),
            variant("temperature", "Temperature", &[TARGET, num("value", "Temperature", D::Temperature).help("The prescribed temperature.").req()]),
        ],
    )
    .help("What kind of load: forces, pressures, gravity, heat and more.")
    .req()
    .def("\"force\""),
];

const FACE_CONDITION: &[Variant] = &[
    variant("wall", "Wall", &[]),
    variant("moving_wall", "Moving wall", &[xyz("velocity", "Velocity", D::Velocity).help("The velocity of the moving wall.").req()]),
    variant("slip", "Slip", &[]),
    variant(
        "velocity",
        "Velocity inlet",
        &[
            xyz("velocity", "Velocity", D::Velocity).help("Inflow velocity.").req(),
            choice("profile", "Profile", &[("uniform", "Uniform"), ("parabolic", "Parabolic")])
                .help("Uniform across the inlet, or parabolic (fully developed flow).")
                .def("\"uniform\""),
        ],
    ),
    variant(
        "pressure",
        "Pressure outlet",
        &[num("pressure", "Pressure", D::Stress).help("Pressure at the outlet, relative to the domain's reference.").req().def("0")],
    ),
    variant("periodic", "Periodic", &[]),
];

const fn face(key: &'static str, label: &'static str) -> Field {
    variants(key, label, FACE_CONDITION).hint("wall")
}

pub const FLUID_DOMAIN: &[Field] = &[
    NAME,
    reference("material", "Fluid", C::Material).help("The fluid material: needs a density and a viscosity.").req(),
    xyz("min", "Box min", D::Length).help("The box corner with the smallest coordinates.").req(),
    xyz("max", "Box max", D::Length).help("The box corner with the largest coordinates.").req(),
    positive("spacing", "Cell size", D::Length)
        .help("Lattice cell edge length. Smaller is more accurate and costs as the cube of the refinement.")
        .req(),
    group(
        "faces",
        "Box faces",
        &[
            face("x_min", "X min").help("Condition on the box face at the smallest X."),
            face("x_max", "X max").help("Condition on the box face at the largest X."),
            face("y_min", "Y min").help("Condition on the box face at the smallest Y."),
            face("y_max", "Y max").help("Condition on the box face at the largest Y."),
            face("z_min", "Z min").help("Condition on the box face at the smallest Z."),
            face("z_max", "Z max").help("Condition on the box face at the largest Z."),
        ],
    )
    .help("The condition on each face of the box."),
    references("obstacles", "Obstacles", C::Body).help("Bodies of the design the flow goes around."),
    json("surfaces", "Obstacle faces")
        .help("Conditions on single obstacle faces, as JSON; walls by default.")
        .hint("[{\"face\", \"condition\"}]; walls by default"),
    xyz("initial_velocity", "Initial velocity", D::Velocity).help("The fluid's velocity at the start."),
    xyz("acceleration", "Acceleration", D::Acceleration).help("A body acceleration on the fluid, such as gravity."),
    positive("tolerance", "Obstacle tolerance", D::Length)
        .help("How close a cell must be to an obstacle surface to count as on it.")
        .hint("a tenth of the cell size"),
];

pub const FLUID_BOUNDARY: &[Field] = &[
    reference("domain", "Domain", C::FluidDomain).help("The fluid domain to change.").req(),
    json("at", "Boundary")
        .help("A box face (x_min … z_max) or an obstacle face, as JSON.")
        .req()
        .def("\"x_min\"")
        .hint("\"x_min\" … \"z_max\", or {\"body\", \"face\"}"),
    variants("condition", "Condition", FACE_CONDITION).help("The flow condition on that boundary.").req().def(r#"{"type": "velocity"}"#),
];

const PROCEDURE: Field = variants(
    "procedure",
    "Procedure",
    &[
        variant("static", "Static, linear", &[]),
        variant(
            "nonlinear_static",
            "Static, nonlinear",
            &[
                flag("nonlinear_geometry", "Nonlinear geometry")
                    .help("Account for large displacements and rotations. Needed for buckling-like and large-deflection problems.")
                    .def("true"),
                range("initial_increment", "Initial increment", D::None, 0.0, 1.0)
                    .help("First increment, as a fraction of the step (0.1 = 10 %).")
                    .def("0.1"),
                range("min_increment", "Min increment", D::None, 0.0, 1.0).help("Smallest increment allowed before the step gives up.").def("1e-5"),
                range("max_increment", "Max increment", D::None, 0.0, 1.0).help("Largest increment, as a fraction of the step.").def("1.0"),
                int("max_increments", "Max increments", 1, 1_000_000).help("The most increments the step may take.").def("100"),
                int("max_iterations", "Iterations per increment", 1, 1000).help("Newton iterations before an increment is cut back.").def("16"),
                group(
                    "tolerances",
                    "Tolerances",
                    &[
                        positive("force", "Force", D::None).help("Residual force, relative to the applied forces.").def("1e-6"),
                        positive("displacement", "Displacement", D::None)
                            .help("Displacement correction, relative to the increment's displacement.")
                            .def("1e-5"),
                        positive("energy", "Energy", D::None).help("Work of the residual, relative to the increment's work.").def("1e-10"),
                    ],
                )
                .help("Convergence tolerances; the defaults suit most models."),
                flag("line_search", "Line search")
                    .help("Scale each Newton correction to reduce the residual; helps with hard convergence.")
                    .def("false"),
                json("load_path", "Load path")
                    .help("How the load factor varies through the step, as JSON pairs of step fraction and factor; a linear ramp when empty.")
                    .hint("[[fraction, factor], …]; a ramp when empty"),
                group(
                    "imperfection",
                    "Imperfection",
                    &[
                        reference("step", "Buckling step", C::Step).help("The solved buckling step whose mode is used.").req(),
                        int("mode", "Mode", 1, 100_000).help("Which buckling mode, from 1.").req().def("1"),
                        num("scale", "Largest component", D::Length)
                            .help("The largest displacement of the scaled mode, e.g. 1 % of the thickness.")
                            .req(),
                    ],
                )
                .help("Seed a buckling mode shape into the geometry, for post-buckling analysis."),
                group(
                    "arc_length",
                    "Arc length",
                    &[
                        choice("constraint", "Constraint", &[("cylindrical", "Cylindrical"), ("spherical", "Spherical")])
                            .help("Cylindrical: displacements only. Spherical: displacements and load factor.")
                            .def("\"cylindrical\""),
                        positive("max_load_factor", "Max load factor", D::None).help("Stop when the load factor reaches this.").def("1"),
                        json("displacement_limit", "Displacement limit")
                            .help("Stop when a node's displacement reaches a value, as JSON.")
                            .hint("{\"node\": 1, \"direction\": 3, \"displacement\": -10}"),
                    ],
                )
                .help("Follow the load-displacement path through limit points (snap-through), with the load factor as an unknown."),
            ],
        ),
        variant(
            "buckle",
            "Buckling",
            &[
                int("modes", "Modes", 1, 100_000).help("How many buckling modes to find.").req().def("4"),
                num("shift", "Shift", D::None).help("Look for eigenvalues near this value; 0 finds the lowest."),
            ],
        ),
        variant(
            "frequency",
            "Frequency",
            &[
                int("modes", "Modes", 1, 10_000).help("How many natural frequencies to find.").req().def("10"),
                positive("min_hz", "Min frequency", D::Hertz).help("Lowest frequency of interest."),
                positive("max_hz", "Max frequency", D::Hertz).help("Highest frequency of interest."),
            ],
        ),
        variant(
            "explicit_dynamic",
            "Dynamic, explicit",
            &[
                positive("end_time", "End time", D::Time).help("Duration of the step. An impact is typically a few milliseconds.").req(),
                range("dt_scale", "Time step scale", D::None, 0.0, 1.0)
                    .help("Fraction of the stable time step used, 0 to 1; 0.9 is usual.")
                    .def("0.9"),
                int("max_increments", "Max increments", 1, 10_000_000_000).help("The most time increments the step may take.").def("10000000"),
                positive("output_interval", "Field output every", D::Time)
                    .help("Time between saved result frames; ten frames over the step when empty."),
                positive("history_interval", "History every", D::Time).help("Time between history samples."),
                reference("history_nodes", "History nodes", C::Set).help("The nodes whose displacements are recorded over time."),
                rows(
                    "initial_velocities",
                    "Initial velocities",
                    &[
                        reference("set", "Set", C::Set).help("The set given the velocity.").req(),
                        xyz("velocity", "Velocity", D::Velocity).help("Its starting velocity.").req(),
                    ],
                )
                .help("Starting velocities of sets, for impacts and drops."),
                rows(
                    "rigid_walls",
                    "Rigid walls",
                    &[
                        reference("set", "Set", C::Set).help("The nodes the wall stops.").req(),
                        xyz("point", "Point", D::Length).help("A point on the wall.").req(),
                        xyz("normal", "Normal", D::None).help("The wall's normal, pointing to the side the nodes stay on.").req(),
                        FRICTION,
                    ],
                )
                .help("Flat walls the set's nodes cannot pass."),
                group(
                    "hourglass",
                    "Hourglass control",
                    &[
                        choice("type", "Type", &[("viscous", "Viscous"), ("stiffness", "Stiffness")])
                            .help("Viscous: damping, for high-speed events. Stiffness: springs, for slower ones.")
                            .def("\"viscous\""),
                        positive("coefficient", "Coefficient", D::None).help("Strength of the hourglass control, typically 0.03–0.15."),
                    ],
                )
                .help("Suppresses zero-energy modes of reduced-integration elements."),
                group(
                    "bulk_viscosity",
                    "Bulk viscosity",
                    &[
                        positive("linear", "Linear", D::None).help("Linear bulk viscosity coefficient, typically 0.06."),
                        positive("quadratic", "Quadratic", D::None).help("Quadratic bulk viscosity coefficient, typically 1.2."),
                    ],
                )
                .help("Damps shock-wave ringing."),
                flag("nonlinear_geometry", "Nonlinear geometry").help("Account for large displacements and rotations.").def("true"),
                choice("stress_rate", "Stress rate", &[("jaumann", "Jaumann"), ("green_naghdi", "Green–Naghdi")])
                    .help("How stresses rotate with the material: Jaumann or Green–Naghdi."),
                group(
                    "mass_scaling",
                    "Mass scaling",
                    &[
                        positive("target_increment", "Target increment", D::Time).help("The time step mass scaling aims for.").req(),
                        reference("set", "Set", C::Set).help("Where mass is added; the whole model when empty."),
                    ],
                )
                .help("Adds mass to small elements so the time step reaches the target; check the added mass stays small."),
            ],
        ),
        variant(
            "heat_transfer",
            "Heat transfer",
            &[group(
                "transient",
                "Transient",
                &[
                    positive("end_time", "End time", D::Time).help("Duration of the transient.").req(),
                    positive("increment", "Increment", D::Time).help("Time step of the transient.").req(),
                    range("theta", "θ", D::None, 0.0, 1.0).help("Time integration: 0.5 Crank–Nicolson, 1 backward Euler (default, most stable)."),
                    num("initial_temperature", "Initial temperature", D::Temperature).help("Temperature at the start of the transient."),
                ],
            )
            .help("Solve over time instead of for the steady state.")
            .hint("steady state when off")],
        ),
        variant("coupled_temperature_displacement", "Coupled temperature–displacement", &[]),
        variant(
            "fluid",
            "Fluid (lattice Boltzmann)",
            &[
                reference("domain", "Domain", C::FluidDomain).help("The fluid domain to solve.").req(),
                positive("end_time", "End time", D::Time).help("Simulated flow time, or give Steps.").hint("or Steps"),
                int("steps", "Steps", 1, 1_000_000_000_000).help("Lattice time steps, instead of End time."),
                positive("time_step", "Time step", D::Time)
                    .help("The lattice time step; chosen for a Mach number of 0.087 when empty.")
                    .hint("chosen for Mach 0.087"),
                int("frames", "Frames", 1, 100_000).help("How many result frames to keep.").def("10"),
                positive("magic", "TRT Λ", D::None).help("The two-relaxation-time magic parameter; 0.1875 is usual.").hint("0.1875"),
            ],
        ),
    ],
)
.help("What the step solves: linear or nonlinear static, buckling, frequencies, explicit dynamics, heat transfer or flow.");

const OUTPUT: Field = multi(
    "output",
    "Output",
    &[
        ("\"displacement\"", "Displacement"),
        ("\"reaction_force\"", "Reaction force"),
        ("\"stress\"", "Stress"),
        ("\"strain\"", "Strain"),
        ("\"temperature\"", "Temperature"),
        ("\"heat_flux\"", "Heat flux"),
    ],
)
.help("Which results to keep; by procedure when none is ticked.")
.hint("by procedure");
const STEP_BCS: Field =
    references("boundary_conditions", "Boundary conditions", C::BoundaryCondition).help("The boundary conditions active in the step.");
const STEP_LOADS: Field = references("loads", "Loads", C::Load).help("The loads active in the step.");

pub const STEP_CREATE: &[Field] = &[
    NAME,
    PROCEDURE.req().def(r#"{"type": "static"}"#),
    OUTPUT,
    STEP_BCS,
    STEP_LOADS,
    reference("before", "Insert before", C::Step).help("The step the new one goes before; last when empty.").hint("last"),
];

pub const STEP_EDIT: &[Field] = &[reference("name", "Step", C::Step).help("The step to change.").req(), PROCEDURE, OUTPUT, STEP_BCS, STEP_LOADS];

pub const STEP_ACTIVATE: &[Field] = &[
    reference("step", "Step", C::Step).help("The step to change.").req(),
    references("boundary_conditions", "Boundary conditions", C::BoundaryCondition).help("The boundary conditions to switch on or off in the step."),
    references("loads", "Loads", C::Load).help("The loads to switch on or off in the step."),
    flag("active", "Active").help("On: they act in the step. Off: they are switched off from it on.").def("true"),
];

pub const DATUM_SUPPRESS: &[Field] = &[
    reference("name", "Datum", C::Datum).help("The datum to keep but leave out of the model.").req(),
    flag("suppressed", "Suppressed").help("On: left out (its partitions stop cutting). Off: back in the model.").def("true"),
];

pub const PARTITION_SUPPRESS: &[Field] = &[
    reference("name", "Partition", C::Partition).help("The partition to keep but leave out: its cells merge back.").req(),
    flag("suppressed", "Suppressed").help("On: the design is cut without it. Off: it cuts again.").def("true"),
];

pub const RENAME: &[Field] = &[
    choice(
        "kind",
        "Kind",
        &[
            ("set", "Set"),
            ("material", "Material"),
            ("section", "Section"),
            ("step", "Step"),
            ("boundary_condition", "Boundary condition"),
            ("load", "Load"),
            ("reference_point", "Reference point"),
            ("coupling", "Coupling"),
            ("contact", "Contact"),
            ("tie", "Tie"),
            ("point_mass", "Point mass"),
            ("spring", "Spring"),
            ("fluid_domain", "Fluid domain"),
            ("datum", "Datum"),
        ],
    )
    .help("What kind of definition to rename.")
    .req(),
    text("from", "Name").help("Its current name.").req(),
    text("to", "New name").help("Every definition that refers to it is changed to the new name.").req(),
];

pub const UNITS: &[Field] = &[choice(
    "system",
    "Unit system",
    &[
        ("mm-t-s-N-MPa", "mm, t, s, N, MPa"),
        ("m-kg-s-N-Pa", "m, kg, s, N, Pa"),
        ("mm-kg-ms-kN-GPa", "mm, kg, ms, kN, GPa"),
        ("mm-g-ms-N-MPa", "mm, g, ms, N, MPa"),
        ("in-lbf-s-psi", "in, lbf·s²/in, s, lbf, psi"),
        ("unspecified", "Unspecified"),
    ],
).help("The units the model's numbers are in. FieldCraft never converts: enter every value in this system. Designs from SolidCraft are in mm, so mm-t-s-N-MPa fits them.")
.req()
.def("\"mm-t-s-N-MPa\"")];

// ---- Contact ----

const FRICTION: Field = group(
    "friction",
    "Friction",
    &[
        positive("static", "Static μ", D::None)
            .help("Friction coefficient at rest. Steel on steel ≈ 0.15–0.8, rubber on concrete ≈ 0.6–1.")
            .hint("0"),
        positive("kinetic", "Kinetic μ", D::None)
            .help("Friction coefficient while sliding, usually a little lower than the static.")
            .hint("the static μ"),
        positive("decay", "Decay", D::None).help("How fast friction falls from static to kinetic with slip rate.").hint("0; per slip rate"),
    ],
)
.help("Coulomb friction between the surfaces; frictionless when off.")
.hint("frictionless");
const PENALTY: Field = range("penalty_scale", "Penalty scale", D::None, 0.0, 1.0)
    .help("Scales the contact stiffness. Lower softens contact and helps convergence, but lets surfaces interpenetrate more.")
    .def("1");
const PENETRATION: Field = choice("initial_penetration", "Initial penetration", &[("adjust", "Adjust nodes"), ("refuse", "Refuse")])
    .help("Adjust: move secondary nodes onto the main surface at the start. Refuse: report overlaps as errors.")
    .def("\"adjust\"");

pub const CONTACT_PAIR: &[Field] = &[
    NAME,
    reference("main", "Main surface", C::Set).help("The stiffer or coarser surface.").req(),
    reference("secondary", "Secondary surface", C::Set).help("The finer surface; its nodes are kept out of the main surface.").req(),
    FRICTION,
    PENALTY,
    PENETRATION,
];
pub const CONTACT_GENERAL: &[Field] = &[
    NAME,
    reference("surface", "Surface", C::Set).help("The faces that may touch each other; every exterior face when empty.").hint("every exterior face"),
    FRICTION,
    PENALTY,
    PENETRATION,
];
pub const CONTACT_EDIT: &[Field] = &[
    reference("name", "Contact", C::Contact).help("The contact to change.").req(),
    variants(
        "surfaces",
        "Surfaces",
        &[
            variant(
                "pair",
                "Pair",
                &[
                    reference("main", "Main", C::Set).help("The stiffer or coarser surface.").req(),
                    reference("secondary", "Secondary", C::Set).help("The finer surface.").req(),
                ],
            ),
            variant(
                "general",
                "General",
                &[reference("surface", "Surface", C::Set)
                    .help("The faces that may touch; every exterior face when empty.")
                    .hint("every exterior face")],
            ),
        ],
    )
    .help("A pair of surfaces, or general contact among many faces.")
    .req(),
    FRICTION,
    PENALTY,
    PENETRATION,
];
pub const TIE: &[Field] = &[
    NAME,
    reference("main", "Main surface", C::Set).help("The surface the secondary is tied to; usually the coarser mesh.").req(),
    reference("secondary", "Secondary surface", C::Set).help("The surface whose nodes are tied to the main surface.").req(),
    positive("position_tolerance", "Position tolerance", D::Length)
        .help("Secondary nodes farther than this from the main surface are left untied.")
        .hint("half the main element's thickness"),
];
pub const TIE_EDIT: &[Field] = &[
    reference("name", "Tie", C::Tie).help("The tie to change.").req(),
    reference("main", "Main surface", C::Set).help("The surface the secondary is tied to; usually the coarser mesh.").req(),
    reference("secondary", "Secondary surface", C::Set).help("The surface whose nodes are tied to the main surface.").req(),
    positive("position_tolerance", "Position tolerance", D::Length)
        .help("Secondary nodes farther than this from the main surface are left untied.")
        .hint("half the main element's thickness"),
];
pub const CONTACT_FIND: &[Field] = &[
    positive("tolerance", "Tolerance", D::Length).help("Faces closer than this count as touching.").hint("a tenth of the mean face size"),
    flag("create", "Create pairs").help("Also create a contact for each touching pair found.").def("false"),
    FRICTION,
    PENALTY,
    PENETRATION,
];

// ---- Meshing ----

const BODY: Field =
    reference("body", "Body", C::Body).help("The body to mesh; every body of the design when empty.").hint("every body of the design");
const SURFACE: Field =
    int("surface", "Imported surface", 0, 1_000_000).help("An imported surface mesh to mesh instead of a design body.").hint("instead of a body");
const TOLERANCE: Field = positive("tolerance", "Chordal tolerance", D::Millimetre)
    .help("Largest distance between the tessellation and the true curved surface. Smaller follows curves closer, with more elements.")
    .hint("1e-3 of the diagonal");
const WELD: Field = range("weld", "Weld", D::None, 0.0, 1e-3)
    .help("Points closer than this fraction of the diagonal are merged when the surface is assembled.")
    .hint("1e-9 of the diagonal");
const WELD_DISTANCE: Field = positive("weld_distance", "Weld distance", D::Millimetre)
    .help("Points closer than this are merged when the surface is assembled.")
    .hint("the surface's own");
const FEATURE_ANGLE: Field = range("feature_angle", "Feature angle", D::Degree, 0.0, 180.0)
    .help("Edges where faces meet at more than this angle are kept as sharp features.")
    .hint("45");

pub const TET: &[Field] = &[
    BODY,
    SURFACE,
    positive("size", "Element size", D::Millimetre)
        .help("Target element edge length. Several elements through the thinnest wall are needed for accurate stresses.")
        .hint("1/20 of the diagonal"),
    choice("order", "Order", &[("linear", "Linear (tet4)"), ("quadratic", "Quadratic (tet10)")])
        .help("Linear (tet4) is fast but stiff in bending; quadratic (tet10) is far more accurate for stress.")
        .def("\"linear\""),
    TOLERANCE,
    flag("quality", "Improve quality").help("Smooth and swap elements after meshing to remove slivers.").def("true"),
    range("quality_target", "Quality target", D::Degree, 5.0, 60.0).help("The smallest dihedral angle quality improvement aims for.").hint("40"),
    range("curvature_angle", "Curvature angle", D::Degree, 0.0, 90.0)
        .help("Angle an element may span on a curved surface; smaller refines curved regions.")
        .hint("22.5"),
    range("grading", "Grading", D::None, 0.05, 2.0).help("How fast element size may grow from small to large; smaller is smoother.").hint("0.3"),
    int("max_passes", "Improvement passes", 0, 200).help("The most quality-improvement passes.").hint("20"),
    int("max_points", "Max points", 1, 20_000_000).help("Cap on the points the mesher creates.").hint("1000000"),
    int("max_steiner", "Max Steiner points", 0, 10_000_000).help("Cap on the points added to recover the surface.").hint("50000"),
    int("max_tets", "Max tetrahedra", 1, 50_000_000).help("Cap on the tetrahedra created.").hint("5000000"),
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

const HEX_SIZE: Field = positive("size", "Element size", D::Millimetre).help("Target element edge length.").hint("1/25 of the diagonal");
const HEX_ORDER: Field =
    int("order", "Order", 1, 2).help("1: linear hex8. 2: quadratic hex20, more accurate in bending.").def("1").hint("1: hex8, 2: hex20");

pub const HEX_GENERATE: &[Field] = &[
    BODY,
    SURFACE,
    HEX_SIZE,
    HEX_ORDER,
    choice(
        "method",
        "Method",
        &[("auto", "Automatic"), ("swept", "Any sweep"), ("extrude", "Extrude"), ("sweep", "Sweep"), ("revolve", "Revolve"), ("mapped", "Mapped")],
    )
    .help(
        "How to build the hexes. Automatic tries, in order, an extrusion, a sweep, a revolution and a mapped block, and \
         uses the first that fits; Any sweep tries the first three.",
    )
    .notes(HEX_METHOD_NOTES)
    .def("\"auto\""),
    TOLERANCE,
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

/// What each structured hex method needs of the body, one line each.
const HEX_METHOD_NOTES: &[(&str, &str)] = &[
    ("auto", "Automatic: the first of Extrude, Sweep, Revolve and Mapped that fits the body."),
    ("swept", "Any sweep: the first of Extrude, Sweep and Revolve that fits the body."),
    ("extrude", "Extrude: the body must be one cross-section carried along a straight direction (levels allowed)."),
    ("sweep", "Sweep: a source face carried to a target face through four-sided linking faces."),
    ("revolve", "Revolve: the body must be one cross-section turned around an axis."),
    ("mapped", "Mapped: the body must have six sides, each with four edges (curved faces allowed)."),
];

pub const HEX_SWEEP: &[Field] = &[
    BODY,
    SURFACE,
    HEX_SIZE,
    HEX_ORDER,
    choice("method", "Method", &[("swept", "Any sweep"), ("extrude", "Extrude"), ("sweep", "Sweep"), ("revolve", "Revolve")])
        .help("How to sweep: any sweep, extrusion, sweep along a path or revolution.")
        .notes(HEX_METHOD_NOTES)
        .def("\"swept\""),
    TOLERANCE,
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

pub const HEX_MAPPED: &[Field] = &[BODY, SURFACE, HEX_SIZE, HEX_ORDER, TOLERANCE, WELD, WELD_DISTANCE, FEATURE_ANGLE];

pub const HEX_BLOCKS: &[Field] = &[
    references("bodies", "Cells", C::Body)
        .help("The cells (bodies) to mesh as blocks; every body of the design when empty.")
        .hint("every body of the design"),
    HEX_SIZE,
    HEX_ORDER,
    TOLERANCE,
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

const CELL_METHODS: &[(&str, &str)] = &[
    ("auto", "Auto (hex, else tet)"),
    ("hex", "Hex"),
    ("swept", "Swept"),
    ("extrude", "Extrude"),
    ("sweep", "Sweep"),
    ("revolve", "Revolve"),
    ("mapped", "Mapped"),
    ("tet", "Tet"),
];

pub const CELLS: &[Field] = &[
    references("bodies", "Cells", C::Body)
        .help("The cells (bodies) to mesh: tick them, or pick them in the viewport (Select: Bodies) and Use selection. Every body of the design when none is ticked.")
        .hint("every body of the design"),
    ref_choices("methods", "Methods", C::Body, CELL_METHODS).help("A meshing method for single cells; the others take Other cells' method."),
    choice(
        "default",
        "Other cells",
        &[("auto", "Auto (hex, else tet)"), ("hex", "Hex"), ("swept", "Swept"), ("mapped", "Mapped"), ("tet", "Tet"), ("blocks", "All hex (conforming blocks)")],
    )
    .help("The method for cells not listed in Methods. All hex meshes every cell as a mapped block, with the division counts of all cells assigned together so shared faces match (mesh.hex.blocks).")
    .notes(&[("blocks", "All hex: every cell must have six sides, each with four edges; leave Methods empty.")])
        .def("\"auto\""),
    HEX_SIZE,
    HEX_ORDER,
    TOLERANCE,
];

pub const CLEAR_MESH: &[Field] = &[references("bodies", "Bodies", C::Body)
    .help("The bodies whose meshes go. When none is ticked: the meshes of bodies the open design no longer has because its partitions changed (model.check lists them).")
    .hint("the bodies partitions removed")];

pub const MIRROR: &[Field] = &[
    json("plane", "Plane")
        .help("The mirror plane: a datum plane's name, or a point and normal as JSON.")
        .req()
        .hint(r#""Datum" or {"point": [x, y, z], "normal": [nx, ny, nz]}"#),
    references("bodies", "Bodies", C::Body).help("The meshed bodies to mirror; every meshed body when empty.").hint("every meshed body"),
    json("as", "Images")
        .help("Names for the mirrored bodies, as JSON.")
        .hint(r#"{"body": "image body"}; default: the design's body at the reflected place"#),
];

pub const SYMMETRY: &[Field] = &[
    NAME,
    reference("datum", "Datum plane", C::DatumPlane).help("The symmetry plane.").req(),
    reference("set", "Set", C::Set).help("The faces on the plane; found from the design when empty.").hint("the faces of the design on the plane"),
    references("bodies", "Bodies", C::Body).help("The bodies cut by the symmetry plane; every body when empty.").hint("every body"),
];

pub const HEX_OCTREE: &[Field] = &[
    BODY,
    SURFACE,
    HEX_SIZE,
    TOLERANCE,
    int("max_depth", "Refinement levels", 0, 5).help("How many times the octree may refine near the surface.").hint("3"),
    int("max_elements", "Max elements", 1, 10_000_000).help("Cap on the elements created."),
    int("attempts", "Attempts", 1, 8).help("How many times to retry with adjusted settings.").hint("4"),
    int("max_sweeps", "Optimiser sweeps", 0, 10_000).help("The most smoothing sweeps.").hint("300"),
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

pub const BLOCK: &[Field] = &[
    text("body", "Body").help("Name of the new block body.").req().def("\"Block\""),
    choice(
        "kind",
        "Element",
        &[
            ("hex8", "hex8"),
            ("hex20", "hex20"),
            ("tet4", "tet4"),
            ("tet10", "tet10"),
            ("wedge6", "wedge6"),
            ("quad4", "quad4 (2D)"),
            ("quad8", "quad8 (2D)"),
            ("tri3", "tri3 (2D)"),
            ("tri6", "tri6 (2D)"),
        ],
    )
    .help("The element type of the block.")
    .req()
    .def("\"hex8\""),
    points("corners", "Corners", D::Length, 8).help("The eight corner points (four for 2D, at z = 0).").req(),
    json("divisions", "Divisions").help("Elements along each edge direction, as JSON.").req().hint("[n12, n14, n15] (2D: [n12, n14])"),
    json("bias", "Bias").help("Element size ratio along each direction, as JSON; 1 is uniform.").hint("[1, 1, 1]"),
    json("edges", "Curved and named edges").help("Edge curves and names, as JSON."),
    json("faces", "Face names").help("Names of the block's faces, as JSON.").hint("{\"1\": \"bottom\"}"),
    json("vertices", "Corner names").help("Names of the block's corners, as JSON."),
    text("face", "2D face name").help("Name of a 2D block's face.").hint("area"),
    positive("merge", "Merge distance", D::Length).help("Nodes closer than this merge with existing nodes.").hint("1e-6 of the diagonal"),
];

pub const PLANAR: &[Field] = &[
    positive("size", "Element size", D::Length).help("Target element edge length.").req(),
    choice("kind", "Element", &[("quad4", "quad4"), ("quad8", "quad8"), ("tri3", "tri3"), ("tri6", "tri6")])
        .help("The 2D element type.")
        .def("\"quad4\""),
    BODY,
    text("face", "Face").help("A planar face of the body to mesh.").hint("a planar face of the body"),
    TOLERANCE,
    json("polygon", "Polygon").help("An outline to mesh instead of a face, as JSON points.").hint("[[x, y], …] instead of a face"),
    json("holes", "Holes").help("Holes in the polygon, as JSON point lists."),
    text("name", "Body name").help("Name of the new 2D body.").hint("Plane"),
    int("max_elements", "Max elements", 1, 4_000_000).help("Cap on the elements created."),
];

pub const QUALITY: &[Field] = &[
    labels("elements", "Elements", LabelOf::Element).help("The elements to measure; all when empty.").hint("all"),
    int("bins", "Histogram bins", 1, 1000).help("Number of bars in the quality histogram.").def("10"),
];

pub const SURFACE_CHECK: &[Field] = &[
    reference("body", "Body", C::Body).help("The body whose surface to check.").req().hint("or an imported surface"),
    SURFACE,
    TOLERANCE,
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

pub const NODE_ADD: &[Field] = &[
    int("label", "Label", 1, u32::MAX as i64).help("The node label.").req(),
    xyz("xyz", "Position", D::Length).help("The node's coordinates.").req().def("[0, 0, 0]"),
];
pub const LABEL: &[Field] = &[int("label", "Label", 1, u32::MAX as i64).help("The label of the node or element.").req()];
pub const ELEMENT_ADD: &[Field] = &[
    int("label", "Label", 1, u32::MAX as i64).help("The element label.").req(),
    choice(
        "kind",
        "Kind",
        &[
            ("rod2", "rod2"),
            ("beam2", "beam2"),
            ("tri3", "tri3"),
            ("tri6", "tri6"),
            ("quad4", "quad4"),
            ("quad8", "quad8"),
            ("tet4", "tet4"),
            ("tet10", "tet10"),
            ("wedge6", "wedge6"),
            ("hex8", "hex8"),
            ("hex20", "hex20"),
        ],
    )
    .help("The element type.")
    .req()
    .def("\"hex8\""),
    labels("nodes", "Nodes", LabelOf::Node).help("The element's node labels in order.").req().hint("labels in order: 1, 2, 3, 4"),
];

const MERGE_TOLERANCE: Field =
    positive("tolerance", "Tolerance", D::Length).help("Nodes closer than this are one node.").hint("a millionth of the mesh's size");

pub const NODE_MERGE: &[Field] = &[
    MERGE_TOLERANCE,
    reference("set", "Set", C::Set).help("Merge only the nodes of this set (and of its elements and faces).").hint("every node"),
    choice("keep", "Keep", &[("lowest", "Lowest label"), ("highest", "Highest label")]).help("Which node of each group survives.").def("\"lowest\""),
];

pub const CHECK_DUPLICATES: &[Field] = &[MERGE_TOLERANCE];

pub const NODE_RELAX: &[Field] = &[
    xyz("center", "Centre", D::Length).help("The centre of the sphere of influence; click the mesh to place it.").hint("click the mesh"),
    positive("radius", "Radius", D::Length).help("Nodes closer to the centre than this move; Alt + scroll over the view changes it."),
    int("around_element", "Around element", 1, 99_999_999)
        .help("Instead of a centre: a sphere round this element (radius default twice its longest edge).")
        .hint("none"),
    labels("nodes", "Nodes", LabelOf::Node).help("Instead of a sphere: these nodes.").hint("none"),
    references("bodies", "Bodies", C::Body).help("Only the nodes of these bodies.").hint("every body"),
    int("iterations", "Iterations", 1, 100).help("Sweeps over the nodes.").def("5"),
    range("strength", "Strength", D::None, 0.0, 1.0).help("How far each node moves toward its relaxed position, 0 to 1.").def("1"),
    choice("falloff", "Falloff", &[("smooth", "Smooth"), ("linear", "Linear"), ("none", "None")])
        .help("How the strength falls from the centre to the radius, so the edit blends into the mesh.")
        .def("\"smooth\""),
    text("field", "Keep as").help("The relaxation field to keep, to apply again after a remesh.").hint("Relax1, Relax2, …"),
    flag("reapply", "Reapply after remesh").help("Apply the field again by itself after each remesh.").def("false"),
];

pub const NODE_RELAX_APPLY: &[Field] = &[
    text("field", "Field").req().help("The relaxation field kept by Relax Nodes."),
    references("bodies", "Bodies", C::Body).help("Only the nodes of these bodies.").hint("the field's bodies"),
    range("polish", "Polish", D::None, 0.0, 1.0).help("Strength of one guarded smoothing sweep after the field is applied; 0 for none.").def("0.3"),
];

pub const RENUMBER: &[Field] = &[
    choice("what", "Renumber", &[("both", "Nodes and elements"), ("nodes", "Nodes"), ("elements", "Elements")])
        .help("Which labels change.")
        .def("\"both\""),
    choice("by", "Order", &[("compact", "Compact (label order)"), ("bandwidth", "Bandwidth (reverse Cuthill–McKee)"), ("body", "Body by body")])
        .help("The order of the new labels.")
        .def("\"compact\""),
    int("start", "Start", 1, 99_999_999).help("The first new label.").def("1"),
    int("offset", "Offset", -99_999_999, 99_999_999).help("Instead of an order and a start: add this to every label.").hint("none"),
];

pub const SEED_EDGE: &[Field] = &[
    json("edges", "Edges")
        .help("The edges to seed, as JSON; easiest with Use selection.")
        .req()
        .hint("[{\"body\", \"edge\"}, …] — Use selection with Select: Edges"),
    int("count", "Count", 1, 100_000).help("Number of elements along each edge.").hint("or an element size"),
    positive("size", "Element size", D::Length).help("Element length along the edge, instead of a count.").hint("or a count"),
    group(
        "bias",
        "Bias",
        &[
            range("ratio", "Ratio", D::None, 1.0, 1e4).help("Largest element over smallest; 1 is uniform.").req().def("1"),
            choice("toward", "Small toward", &[("start", "Start"), ("end", "End"), ("both", "Both ends")])
                .help("Which end gets the small elements.")
                .req()
                .def("\"end\""),
        ],
    )
    .help("Grade the elements along the edge."),
    choice("constraint", "Constraint", &[("exact", "Exact"), ("min", "At least"), ("free", "Free (a goal)")])
        .help("Exact: the mesher must match the count. At least: it may add more. Free: a goal only.")
        .def("\"exact\""),
];

pub const SEED_CLEAR: &[Field] = &[json("edges", "Edges").help("The edges to clear, as JSON; every seed when empty.").hint("every seed")];

pub const DELAUNAY_POINTS: &[Field] = &[points("points", "Points", D::Length, 20_000).help("The points to triangulate.").req()];

pub const TAG: &[Field] = &[
    group(
        "geometry",
        "Geometry",
        &[
            reference("body", "Body", C::Body).help("A body of the open design.").req(),
            text("face", "Face").help("A persistent face name."),
            text("edge", "Edge").help("A persistent edge name."),
            text("vertex", "Vertex").help("A persistent vertex name."),
        ],
    )
    .help("The geometry the mesh entities are tagged with.")
    .req(),
    labels("elements", "Elements", LabelOf::Element).help("Element labels."),
    json("faces", "Element faces").help("Element faces, as JSON."),
    json("edges", "Element edges").help("Element edges, as JSON."),
    labels("nodes", "Nodes", LabelOf::Node).help("Node labels."),
];

// ---- Geometry and files ----

pub const OPEN_DESIGN: &[Field] = &[
    path("path", "Design", false).help("The SolidCraft design file to open.").req(),
    choice("model", "Model", &[("keep", "Keep the current model"), ("new", "Start a new model")])
        .help("Keep the current model with the new geometry, or start a new one.")
        .def("\"keep\""),
];
const POINT: Dim = D::Millimetre;
pub const DATUM_PLANE: &[Field] = &[
    NAME,
    variants(
        "by",
        "Definition",
        &[
            variant(
                "offset",
                "Principal plane",
                &[
                    choice("plane", "Plane", &[("yz", "YZ (normal X)"), ("zx", "ZX (normal Y)"), ("xy", "XY (normal Z)")])
                        .help("The principal plane.")
                        .req(),
                    num("offset", "Offset", POINT).help("Distance along the plane's normal.").def("0"),
                ],
            ),
            variant(
                "three_points",
                "Three points",
                &[
                    xyz("a", "Point A", POINT).help("First point.").req(),
                    xyz("b", "Point B", POINT).help("Second point.").req(),
                    xyz("c", "Point C", POINT).help("Third point.").req(),
                ],
            ),
            variant(
                "point_normal",
                "Point and normal",
                &[
                    xyz("point", "Point", POINT).help("A point on the plane.").req(),
                    xyz("normal", "Normal", D::None).help("The plane's normal direction.").req(),
                ],
            ),
            variant(
                "datum_points",
                "Three datum points",
                &[
                    reference("a", "Point A", C::DatumPoint).help("First datum point.").req(),
                    reference("b", "Point B", C::DatumPoint).help("Second datum point.").req(),
                    reference("c", "Point C", C::DatumPoint).help("Third datum point.").req(),
                ],
            ),
            variant(
                "normal_to_axis",
                "Normal to an axis",
                &[
                    reference("axis", "Axis", C::DatumAxis).help("The datum axis the plane is normal to.").req(),
                    reference("point", "Through", C::DatumPoint)
                        .help("A datum point on the plane; the axis's own point when empty.")
                        .hint("the axis's point"),
                ],
            ),
        ],
    )
    .help("How the plane is defined.")
    .req()
    .def(r#"{"type": "offset"}"#),
];
pub const DATUM_AXIS: &[Field] = &[
    NAME,
    variants(
        "by",
        "Definition",
        &[
            variant(
                "principal",
                "Principal axis",
                &[
                    choice("axis", "Axis", &[("x", "X"), ("y", "Y"), ("z", "Z")]).help("The principal direction.").req(),
                    xyz("through", "Through", POINT).help("A point the axis passes through; the origin when empty.").hint("the origin"),
                ],
            ),
            variant(
                "two_points",
                "Two points",
                &[
                    xyz("a", "Point A", POINT).help("First point on the axis.").req(),
                    xyz("b", "Point B", POINT).help("Second point on the axis.").req(),
                ],
            ),
            variant(
                "point_direction",
                "Point and direction",
                &[
                    xyz("point", "Point", POINT).help("A point on the axis.").req(),
                    xyz("direction", "Direction", D::None).help("The axis direction.").req(),
                ],
            ),
            variant(
                "datum_points",
                "Two datum points",
                &[
                    reference("a", "Point A", C::DatumPoint).help("First datum point on the axis.").req(),
                    reference("b", "Point B", C::DatumPoint).help("Second datum point on the axis.").req(),
                ],
            ),
        ],
    )
    .help("How the axis is defined.")
    .req()
    .def(r#"{"type": "principal"}"#),
];
pub const RIGID_WALL: &[Field] = &[
    reference("step", "Step", C::Step)
        .help("The explicit dynamics step the wall acts in; the model's only one when empty.")
        .hint("the explicit step"),
    reference("set", "Set", C::Set).help("The nodes the wall stops: pick them and Use selection.").req(),
    xyz("point", "Point", D::Length).help("A point on the wall.").req().def("[0, 0, 0]"),
    xyz("normal", "Normal", D::None).help("The wall's normal, pointing to the side the nodes stay on.").req().def("[0, 0, 1]"),
    FRICTION,
];
pub const RIGID_WALL_DELETE: &[Field] = &[
    reference("step", "Step", C::Step).help("The explicit dynamics step.").req(),
    int("index", "Wall", 0, MAX_ROWS as i64).help("The wall's place in the step's rigid walls, from 0.").req(),
];
pub const CENTRE: &[Field] = &[
    rows(
        "geometry",
        "Geometry",
        &[
            reference("body", "Body", C::Body).help("A body of the open design.").req(),
            text("face", "Face").help("A persistent face name of the body."),
            text("edge", "Edge").help("A persistent edge name of the body."),
            text("vertex", "Vertex").help("A persistent vertex name of the body."),
        ],
    )
    .help("Faces or edges (a hole's faces or rim give its centre), or one vertex."),
    labels("nodes", "Nodes", LabelOf::Node).help("Mesh node labels: their mean.").hint("labels: 1, 2, 10-20"),
];
pub const DATUM_POINT: &[Field] = &[
    NAME,
    variants(
        "by",
        "Definition",
        &[
            variant("coordinates", "Coordinates", &[xyz("point", "Point", POINT).help("The point's coordinates.").req()]),
            variant(
                "vertex",
                "Vertex",
                &[
                    reference("body", "Body", C::Body).help("The body the vertex belongs to.").req(),
                    text("vertex", "Vertex")
                        .help("The vertex's persistent name: pick it with Select: Vertices and Use selection (geometry.bodies lists every body's vertices).")
                        .req(),
                ],
            ),
            variant(
                "edge",
                "On an edge",
                &[
                    reference("body", "Body", C::Body).help("The body the edge belongs to.").req(),
                    text("edge", "Edge").help("The edge's persistent name.").req(),
                    range("fraction", "Fraction", D::None, 0.0, 1.0).help("How far along the edge from its start: 0.5 is its midpoint.").def("0.5"),
                ],
            ),
            variant(
                "face_centre",
                "Face centre",
                &[
                    reference("body", "Body", C::Body).help("The body the face belongs to.").req(),
                    text("face", "Face").help("The face's persistent name.").req(),
                ],
            ),
            variant(
                "axis_plane",
                "Axis and plane",
                &[
                    reference("axis", "Axis", C::DatumAxis).help("The datum axis.").req(),
                    reference("plane", "Plane", C::DatumPlane).help("The datum plane the axis crosses.").req(),
                ],
            ),
        ],
    )
    .help("How the point is defined.")
    .req()
    .def(r#"{"type": "coordinates"}"#),
];
pub const VIEW_ITEMS: &[Field] = &[rows(
    "items",
    "Items",
    &[
        choice(
            "kind",
            "Kind",
            &[
                ("body", "Body"),
                ("surface", "Surface"),
                ("datum", "Datum"),
                ("boundary_condition", "Boundary condition"),
                ("load", "Load"),
                ("reference_point", "Reference point"),
                ("coupling", "Coupling"),
            ],
        )
        .help("What kind of item it is.")
        .req(),
        text("name", "Name").help("The item's name.").req(),
    ],
)
.help("The viewport items.")
.req()];
pub const DATUM_DELETE: &[Field] = &[
    reference("name", "Datum", C::Datum).req().help("The Datum to act on, by name."),
    flag("cascade", "Delete its partitions").help("Also delete the partitions that cut by it; without it, a datum in use is refused."),
];
pub const PARTITION: &[Field] = &[
    NAME,
    reference("datum", "Datum plane", C::DatumPlane).help("The plane that divides the bodies.").req(),
    references("bodies", "Bodies", C::Body).help("The bodies to divide; every body the plane cuts when empty.").hint("every body the plane divides"),
];
pub const PARTITION_DELETE: &[Field] = existing!("Partition", C::Partition);
pub const TESSELLATE: &[Field] = &[reference("body", "Body", C::Body).help("The body to tessellate.").req(), TOLERANCE];
pub const IMPORT_MESH: &[Field] = &[path("path", "3MF file", false).help("A 3MF surface mesh with named faces.").req()];
pub const FILE_OPEN: &[Field] = &[path("path", "Model", false).help("The FieldCraft model file to open.").req()];
pub const FILE_SAVE: &[Field] = &[path("path", "File", true).help("Where to save; the model's own file when empty.").hint("the model's file")];
pub const FILE_SAVE_AS: &[Field] = &[path("path", "File", true).help("The file to save to.").req()];
pub const RECOVERY_LIST: &[Field] =
    &[path("dir", "Folder", false).help("The recovery folder; the default one when empty.").hint("the recovery folder")];
pub const CRASH_REPORTS: &[Field] = &[
    int("limit", "Limit", 1, 1000).help("At most this many reports, newest first.").hint("20"),
    path("dir", "Folder", false).help("The crash folder; the default one when empty.").hint("the crash folder"),
];
pub const CRASH_REPORT: &[Field] = &[
    text("id", "Report").help("The crash report to show.").req(),
    path("dir", "Folder", false).help("The crash folder; the default one when empty.").hint("the crash folder"),
];
pub const CRASH_REPORTS_DELETE: &[Field] = &[
    text("id", "Report").help("The crash report to delete.").hint("or All"),
    flag("all", "All").help("Delete every crash report instead of one."),
    path("dir", "Folder", false).help("The crash folder; the default one when empty.").hint("the crash folder"),
];
pub const BUG_REPORT: &[Field] = &[
    text("title", "Title").help("The issue's title; one is made up when empty."),
    text("observed", "What happened").help("What went wrong; the last error when empty."),
    text("crash", "Crash report").help("A crash report to report (its id)."),
    flag("share_session", "Share my session").help("Attach the commands that built the model (they can name your files).").def("false"),
    flag("include_model", "Include the model").help("Attach the model file too (with Share my session).").def("false"),
];
pub const CRASH_TEST: &[Field] =
    &[choice("mode", "Mode", &[("panic", "Panic"), ("recovered", "Recovered panic"), ("abort", "Abort")]).help("How to crash.").req()];
pub const RECOVER: &[Field] = &[
    text("id", "Autosave").help("The autosave to recover.").req(),
    path("dir", "Folder", false).help("The recovery folder; the default one when empty.").hint("the recovery folder"),
];
pub const RECOVERY_DISCARD: &[Field] = &[
    text("id", "Autosave").help("The autosave to discard."),
    flag("all", "All").help("Discard every autosave."),
    path("dir", "Folder", false).help("The recovery folder; the default one when empty.").hint("the recovery folder"),
];
pub const SAMPLE: &[Field] = &[choice(
    "name",
    "Sample",
    &[
        ("cantilever", "Cantilever"),
        ("impact", "Impact"),
        ("parts", "Parts"),
        ("ibeam", "I-beam cantilever"),
        ("drop-test", "Drop test"),
        ("rubber-seal", "Rubber seal"),
        ("flow-past", "Flow past a bracket"),
        ("heated-bracket", "Heated bracket"),
        ("bracket-hex", "Hex-meshed bracket"),
        ("crank-arm", "Bicycle crank arm"),
        ("crank-mirror", "Mirrored crank arm"),
    ],
)
.help("The sample model to open.")
.req()
.def("\"cantilever\"")];
pub const COUNT: &[Field] = &[int("count", "Count", 1, 1_000_000).help("How many steps.").def("1")];

pub const DECK: &[Field] = &[
    choice("format", "Format", &[("abaqus", "Abaqus input (.inp)"), ("nastran", "Nastran bulk data (.bdf)"), ("lsdyna", "LS-DYNA keyword (.k)")])
        .help("The solver deck format.")
        .req()
        .def("\"abaqus\""),
    path("path", "File", true).help("Where to write the deck.").req(),
    references("steps", "Steps", C::Step).help("The steps to write; all when empty.").hint("all"),
];

// ---- Solving ----

const STEP: Field = reference("step", "Step", C::Step).help("The step to solve; the only one when empty.").hint("the only one");
const SOLVED: Field = reference("step", "Step", C::SolvedStep).help("The solved step; the step shown when empty.").hint("the step shown");
const MAX_DOFS: Field = int("max_dofs", "Max DOFs", 1, 6_000_000).help("Cap on the degrees of freedom solved.").hint("6000000");
const MAX_FACTOR: Field = int("max_factor_entries", "Max factor entries", 1, super::solve::FACTOR_ENTRY_CEILING as i64)
    .help("Cap on factor entries (8 bytes each): 400M by default; raise deliberately up to 2B (16 GB of values alone). Indices, workspace and other solver storage need additional memory.")
    .hint("400000000");
const THREADS: Field = int("threads", "Threads", 1, 256).help("Threads to use; the session's setting when empty.").hint("the session's");

pub const SOLVE_STATIC: &[Field] = &[STEP, MAX_DOFS, MAX_FACTOR, THREADS];
pub const SOLVE_NONLINEAR: &[Field] = &[
    STEP,
    MAX_DOFS,
    MAX_FACTOR,
    int("max_newton_iterations", "Max Newton iterations", 1, 100_000).help("Cap on the Newton iterations over the whole step.").hint("100000"),
    THREADS,
];
pub const SOLVE_FREQUENCY: &[Field] = &[
    STEP,
    choice("mass", "Mass matrix", &[("consistent", "Consistent"), ("lumped", "Lumped")])
        .help("Consistent is more accurate; lumped is cheaper.")
        .def("\"consistent\""),
    num("shift", "Shift", D::None).help("Look for frequencies near this eigenvalue; 0 finds the lowest.").hint("0"),
    MAX_DOFS,
    MAX_FACTOR,
    int("max_iterations", "Max Lanczos vectors", 1, 3000).help("Cap on the Lanczos vectors.").hint("3000"),
    THREADS,
];
pub const SOLVE_BUCKLE: &[Field] =
    &[STEP, MAX_DOFS, MAX_FACTOR, int("max_iterations", "Max Lanczos vectors", 1, 3000).help("Cap on the Lanczos vectors.").hint("3000"), THREADS];
pub const SOLVE_EXPLICIT: &[Field] = &[
    STEP,
    MAX_DOFS,
    int("max_increments", "Max increments", 1, 1_000_000_000).help("Cap on the time increments.").hint("1000000000"),
    int("max_output_values", "Max output values", 1, 50_000_000).help("Cap on the result values kept.").hint("50000000"),
    int("max_contact_faces", "Max contact faces", 1, 10_000_000).help("Cap on the faces in contact search.").hint("10000000"),
    THREADS,
];
pub const SOLVE_HISTORY: &[Field] = &[SOLVED.req(), int("every", "Keep one in", 1, 1_000_000_000).help("Keep one history sample in this many.")];
pub const SOLVE_FLUID: &[Field] = &[
    STEP,
    int("max_cells", "Max cells", 1, 16_000_000).help("Cap on the lattice cells."),
    int("max_steps", "Max steps", 1, 10_000_000).help("Cap on the lattice time steps."),
    positive("max_cell_updates", "Max cell updates", D::None).help("Cap on cells × steps."),
    positive("max_frame_bytes", "Max frame bytes", D::None).help("Cap on the memory for result frames."),
    THREADS,
];
pub const SOLVE_PROBE: &[Field] = &[
    SOLVED.req(),
    labels("nodes", "Nodes", LabelOf::Node).help("Node labels to read.").hint("labels: 1, 2, 10-20"),
    reference("set", "Set", C::Set).help("A set whose nodes to read."),
    points("at", "Points", D::Length, 10_000).help("Points to read at the nearest nodes."),
    int("mode", "Mode", 1, 1_000_000).help("The mode to read, for frequency and buckling steps."),
    int("frame", "Frame", 0, 1_000_000_000).help("The frame to read, for explicit and transient steps."),
    flag("elements", "Element results").help("Also read element results (stresses) around the nodes."),
    points("points", "Fluid points", D::Length, 10_000).help("Points in the fluid to read."),
    json("forces", "Fluid walls").help("Walls whose fluid forces to sum, as JSON.").hint("[\"x_min\", \"F2:top\"]"),
];

// ---- Results ----

pub const DEFORMED: &[Field] = &[
    SOLVED,
    flag("show", "Show").help("Show the deformed shape.").def("true"),
    number_or_auto("scale", "Scale", D::None, Some(0.0)).help("Displacement magnification: auto, or a factor (1 is true scale).").def("\"auto\""),
];
pub const CONTOUR: &[Field] = &[
    SOLVED,
    choice(
        "field",
        "Field",
        &[
            ("von_mises", "Von Mises stress"),
            ("displacement", "Displacement"),
            ("displacement_x", "Displacement X"),
            ("displacement_y", "Displacement Y"),
            ("displacement_z", "Displacement Z"),
            ("max_principal", "Max principal"),
            ("mid_principal", "Mid principal"),
            ("min_principal", "Min principal"),
            ("stress_xx", "Stress XX"),
            ("stress_yy", "Stress YY"),
            ("stress_zz", "Stress ZZ"),
            ("stress_xy", "Stress XY"),
            ("stress_xz", "Stress XZ"),
            ("stress_yz", "Stress YZ"),
            ("contact_pressure", "Contact pressure"),
            ("contact_gap", "Contact gap"),
            ("contact_status", "Contact status"),
            ("contact_shear", "Contact shear"),
            ("contact_slip", "Contact slip"),
            ("none", "None"),
        ],
    )
    .help("The result shown as colours.")
    .req()
    .def("\"von_mises\""),
    int("bands", "Bands", 2, 24).help("Number of colour bands.").def("12"),
    group(
        "range",
        "Legend range",
        &[
            num("min", "Min", D::None).help("Values below it take the lowest colour.").hint("the field's minimum"),
            num("max", "Max", D::None).help("Values above it are drawn in the over colour.").hint("the field's maximum"),
        ],
    )
    .help("Clamp the legend, e.g. below a singular peak; empty follows the field."),
];
pub const MODES: &[Field] = &[SOLVED, int("mode", "Mode", 1, 1_000_000).help("The mode shape shown.").def("1")];
pub const ANIMATE: &[Field] = &[flag("on", "Animate").help("Play the deformation or the frames.").def("true")];
pub const FRAME: &[Field] = &[
    SOLVED,
    int("frame", "Frame", 0, 1_000_000_000).help("The frame shown."),
    positive("time", "Time", D::Time).help("Show the frame nearest this time; the last when empty.").hint("the last frame"),
];
pub const EXPORT_CSV: &[Field] = &[
    choice("what", "Export", &[("reactions", "Reaction sums"), ("probe", "Probed nodes")]).help("What to write.").req().def("\"reactions\""),
    path("path", "File", true).help("The CSV file to write.").req(),
    SOLVED,
    references("sets", "Sets", C::Set).help("Reactions: the sets to sum; the step's BC sets when empty.").hint("the step's BC sets"),
    xyz("about", "About point", D::Length).help("Reactions: the point moments are taken about."),
    labels("nodes", "Nodes", LabelOf::Node).help("Probe: the node labels."),
    reference("set", "Set", C::Set).help("Probe: a set whose nodes to read."),
    points("at", "Points", D::Length, 10_000).help("Probe: points to read at the nearest nodes."),
    int("mode", "Mode", 1, 1_000_000).help("Probe: the mode to read, for frequency and buckling steps."),
    int("frame", "Frame", 0, 1_000_000_000).help("Probe: the frame to read, for explicit and transient steps."),
];
pub const REACTIONS: &[Field] = &[
    SOLVED,
    references("sets", "Sets", C::Set)
        .help("The sets whose reaction forces to sum; the step's boundary-condition sets when empty.")
        .hint("the step's BC sets"),
    xyz("about", "About point", D::Length).help("The point moments are taken about."),
];

#[cfg(test)]
mod tests {
    use fieldcraft_model::UnitSystem;
    use serde_json::{Value, json};

    use super::*;
    use crate::{EngineError, Session};

    /// The form's starting parameters (and those with the required names and references filled
    /// in from the model) are accepted, or refused only for a required field still empty; never
    /// for a value the form put there itself. Errors other than parameter errors (no design, no
    /// results) come after the parameters were read.
    #[test]
    fn form_defaults_are_accepted_or_need_only_required_fields() {
        let (mut checked, mut failures) = (0, Vec::new());
        for c in crate::commands() {
            let Some(form) = c.form else { continue };
            if c.id.starts_with("file.") && c.id != "file.save_as" || c.id == "edit.undo" || c.id == "edit.redo" {
                continue; // they replace or write the session's model; their forms are paths and counts
            }
            for fill_refs in [false, true] {
                let limits = crate::Limits { max_steiner: 10, max_tets: 100, max_hex_elements: 100, threads: 1 };
                let mut s = Session { limits, ..Session::default() };
                crate::cmd::define::tests::defined_bar(&mut s);
                s.execute("solve.run", &json!({})).unwrap();
                let mut p = prefill(form, &json!({}));
                if fill_refs {
                    fill_required(&s, form, &mut p);
                }
                let missing: Vec<String> = problems(form, &p).into_iter().filter(|p| p.kind == ProblemKind::Required).map(|p| p.path).collect();
                match s.execute(c.id, &p) {
                    Err(EngineError::Param(e)) if e.kind == ProblemKind::Required && missing.contains(&e.path) => {}
                    Err(EngineError::Param(e)) => {
                        failures.push(format!("{}: {p} refused: {}, but only {missing:?} were missing", c.id, e.for_agents()));
                    }
                    // State, not parameters: the meshers' default is the open design's bodies, the
                    // bar has no explicit step; activation and probes need one of several fields.
                    Err(EngineError::Params(e)) if STATE.iter().any(|m| e.contains(m)) => {}
                    Err(EngineError::Params(e)) if !missing.iter().any(|k| e.contains(k.rsplit('/').next().unwrap_or(k))) => {
                        failures.push(format!("{}: {p} refused for `{e}`, but only {missing:?} were missing", c.id));
                    }
                    Err(EngineError::Params(_)) => {}
                    _ => checked += 1,
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
        assert!(checked > 20, "{checked}");
    }

    /// Every parameter the engine refuses is a field of the command's form: the problem's path
    /// leads to a field, so a form can show it beside that field, by its label. Each field of
    /// each form is given a value of the wrong kind in turn, over the form's starting values.
    #[test]
    fn every_param_error_names_a_field_of_its_form() {
        let hostile = [json!("x"), json!(-1), json!(0), json!(1e300), json!([]), json!({}), json!(null), json!([1, "x"])];
        let (mut seen, mut failures) = (0, Vec::new());
        for c in crate::commands() {
            let Some(form) = c.form else { continue };
            if c.id.starts_with("file.") || c.id.starts_with("edit.") || form.iter().any(|f| matches!(f.kind, Kind::Path { .. })) {
                continue; // they replace the session's model or write files
            }
            let base = prefill(form, &json!({}));
            let mut tries = vec![json!({}), base.clone()];
            for f in form.iter().filter(|f| !f.key.is_empty()) {
                for v in &hostile {
                    let mut p = base.clone();
                    if let Some(o) = p.as_object_mut() {
                        o.insert(f.key.into(), v.clone());
                    }
                    tries.push(p);
                }
            }
            for p in tries {
                let limits = crate::Limits { max_steiner: 10, max_tets: 100, max_hex_elements: 100, threads: 1 };
                let mut s = Session { limits, ..Session::default() };
                crate::cmd::define::tests::defined_bar(&mut s);
                if let Err(EngineError::Param(e)) = s.execute(c.id, &p) {
                    seen += 1;
                    if e.field.is_none() || field_at(form, &e.path).is_none() {
                        failures.push(format!("{}: {p} refused `{}`, not a field of the form ({})", c.id, e.path, e.for_agents()));
                    }
                }
            }
        }
        failures.sort();
        failures.dedup();
        assert!(failures.is_empty(), "{failures:#?}");
        assert!(seen > 50, "{seen}");
    }

    /// A required field left empty says what to do by its kind: a name, a path, else "required".
    #[test]
    fn required_fields_are_phrased_by_kind() {
        let form = |fields: &'static [Field]| problems(fields, &json!({})).iter().map(Problem::for_form).collect::<Vec<_>>();
        assert_eq!(form(RECOVER), ["Autosave: required"]);
        assert_eq!(form(FILE_SAVE_AS), ["File: enter a path"]);
        assert!(form(MATERIAL).contains(&"Name: enter a name".to_string()), "{:?}", form(MATERIAL));
    }

    /// The commands whose form is the JSON text alone (no schema yet). None: keep it that way.
    #[test]
    fn every_command_has_a_form() {
        let without: Vec<&str> = crate::commands().iter().filter(|c| c.form.is_none()).map(|c| c.id).collect();
        assert_eq!(without, Vec::<&str>::new());
        // A command that takes nothing has an empty form, and says so.
        for c in crate::commands() {
            assert_eq!(c.form.is_some_and(<[Field]>::is_empty), c.params.trim_start().starts_with("{}"), "{}", c.id);
        }
    }

    const STATE: &[&str] = &[
        "open a design first",
        "update checks are off",
        "`boundary_conditions` or `loads`",
        "give one of",
        "is not an explicit",
        "has no explicit dynamics step",
        "has a result",
        "no crash report",
        "FIELDCRAFT_CRASH_TEST",
        "no relaxation field",
    ];

    /// Fill top-level required names (a fresh one) and references (the first existing item).
    fn fill_required(s: &Session, form: &[Field], p: &mut Value) {
        let Some(obj) = p.as_object_mut() else { return };
        for f in form {
            if !f.required || obj.contains_key(f.key) {
                continue;
            }
            match f.kind {
                Kind::Text => {
                    obj.insert(f.key.into(), json!("Fresh"));
                }
                Kind::Ref(c) => {
                    if let Some(n) = s.names(c).first() {
                        obj.insert(f.key.into(), json!(n));
                    }
                }
                _ => {}
            }
        }
    }

    /// Every field of every form says what it means, and every number with a dimension shows
    /// its unit, or its dimension when the model has no unit system.
    #[test]
    fn every_field_has_help_and_a_unit() {
        fn walk(id: &str, fields: &[Field], missing: &mut Vec<String>) {
            for f in fields {
                if f.help.trim().is_empty() {
                    missing.push(format!("{id}: {} ({}) has no help", f.label, f.key));
                }
                if f.dim().is_some_and(|d| d != D::None) {
                    for system in [UnitSystem::Unspecified, UnitSystem::MmTonneS, UnitSystem::MKgS] {
                        if f.unit_label(system).is_none_or(|u| u.is_empty()) {
                            missing.push(format!("{id}: {} shows no unit in {system:?}", f.label));
                        }
                    }
                }
                match f.kind {
                    Kind::Group(g) | Kind::Rows { fields: g, .. } | Kind::Either(g) => walk(id, g, missing),
                    Kind::Variants { variants, .. } => variants.iter().for_each(|v| walk(id, v.fields, missing)),
                    _ => {}
                }
            }
        }
        let mut missing = Vec::new();
        for c in crate::commands() {
            walk(c.id, c.form.unwrap_or_default(), &mut missing);
        }
        assert!(missing.is_empty(), "{missing:#?}");
        let stress = positive("s", "S", D::Stress);
        assert_eq!(stress.unit_label(UnitSystem::Unspecified).as_deref(), Some("[stress]"));
        assert_eq!(stress.unit_label(UnitSystem::MmTonneS).as_deref(), Some("MPa"));
        assert!(stress.help_text(UnitSystem::Unspecified).contains(UNITS_HINT));
    }

    /// Every default is JSON; every variant field's default names a variant.
    #[test]
    fn defaults_parse() {
        fn walk(fields: &[Field]) {
            for f in fields {
                if let Some(d) = f.default {
                    assert!(serde_json::from_str::<serde_json::Value>(d).is_ok(), "{}: {d}", f.key);
                }
                match f.kind {
                    Kind::Group(g) | Kind::Rows { fields: g, .. } | Kind::Either(g) => walk(g),
                    Kind::Variants { variants, .. } => variants.iter().for_each(|v| walk(v.fields)),
                    _ => {}
                }
            }
        }
        for c in crate::commands() {
            if let Some(form) = c.form {
                walk(form);
            }
        }
    }
}
