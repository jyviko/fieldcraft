//! The commands' parameter schemas ([`crate::schema`]), area by area. Each describes the JSON
//! the command's `run` reads, as its `params` doc string does in words.

use crate::schema::Collection as C;
use crate::schema::Dim as D;
use crate::schema::*;

// ---- Shared pieces ----

/// `{"set": …}` or `{"reference_point": …}`.
const TARGET_OPTIONS: &[Field] = &[reference("set", "Set", C::Set), reference("reference_point", "Reference point", C::ReferencePoint)];
const fn target(key: &'static str, label: &'static str) -> Field {
    either(key, label, TARGET_OPTIONS)
}

/// The name of an existing definition, for delete commands.
macro_rules! existing {
    ($label:literal, $c:expr) => {
        &[reference("name", $label, $c).req()]
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
            reference("body", "Body", C::Body).req(),
            text("face", "Face").hint("persistent face name"),
            text("edge", "Edge"),
            text("vertex", "Vertex"),
        ],
    )
    .hint("whole bodies, or their faces, edges or vertices"),
    json("nodes", "Nodes").hint("[labels]"),
    json("elements", "Elements").hint("[labels]"),
    json("faces", "Element faces").hint("[{\"element\", \"face\"}]"),
    json("edges", "Element edges").hint("[{\"element\", \"edge\"}]"),
];

const ELASTIC: Field = variants(
    "elastic",
    "Elasticity",
    &[
        variant(
            "isotropic",
            "Isotropic",
            &[num("young", "Young's modulus", D::Stress).req(), range("poisson", "Poisson's ratio", D::None, -1.0, 0.5).req()],
        ),
        variant(
            "engineering_constants",
            "Orthotropic",
            &[
                vector("young", "E1, E2, E3", D::Stress, &["E1", "E2", "E3"]).req(),
                vector("poisson", "ν12, ν13, ν23", D::None, &["ν12", "ν13", "ν23"]).req(),
                vector("shear", "G12, G13, G23", D::Stress, &["G12", "G13", "G23"]).req(),
            ],
        ),
    ],
)
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
                positive("yield_stress", "Yield stress", D::Stress).req(),
                num("hardening_modulus", "Hardening modulus", D::Stress).def("0"),
                choice("hardening", "Hardening", HARDENING).def("\"isotropic\""),
            ],
        ),
        variant(
            "table",
            "Table",
            &[rows(
                "points",
                "Hardening curve",
                &[positive("strain", "Plastic strain", D::None).req(), positive("stress", "Stress", D::Stress).req()],
            )
            .req()],
        ),
        variant(
            "johnson_cook",
            "Johnson–Cook",
            &[
                positive("a", "A", D::Stress).req(),
                num("b", "B", D::Stress).req(),
                num("n", "n", D::None).req(),
                num("c", "C", D::None).def("0"),
                num("m", "m", D::None).def("1"),
                positive("reference_strain_rate", "Reference strain rate", D::None).def("1").hint("per time unit"),
                num("room_temperature", "Room temperature", D::Temperature).req(),
                num("melt_temperature", "Melt temperature", D::Temperature).req(),
                positive("specific_heat", "Specific heat", D::SpecificHeat).hint("isothermal when empty"),
                range("taylor_quinney", "Taylor–Quinney", D::None, 0.0, 1.0).def("0.9"),
            ],
        ),
    ],
);

const DAMAGE: Field = variants(
    "damage",
    "Damage",
    &[variant(
        "johnson_cook",
        "Johnson–Cook",
        &[
            num("d1", "D1", D::None).req(),
            num("d2", "D2", D::None).req(),
            num("d3", "D3", D::None).req(),
            num("d4", "D4", D::None),
            num("d5", "D5", D::None),
        ],
    )],
);
const EROSION: Field = group("erosion", "Erosion", &[positive("plastic_strain", "Plastic strain", D::None).req()]);

const HYPERELASTIC: Field = variants(
    "hyperelastic",
    "Hyperelasticity",
    &[
        variant("neo_hookean", "Neo-Hookean", &[]),
        variant("mooney_rivlin", "Mooney–Rivlin", &[num("c01", "C01", D::Stress).req()]),
        variant("yeoh", "Yeoh", &[num("c20", "C20", D::Stress).req(), num("c30", "C30", D::Stress).req()]),
    ],
)
.hint("large strain in nonlinear steps, from the isotropic constants");

const THERMAL: Field = group(
    "thermal",
    "Thermal",
    &[
        json("conductivity", "Conductivity").hint("k or [k1, k2, k3]"),
        positive("specific_heat", "Specific heat", D::SpecificHeat),
        json("expansion", "Expansion").hint("α or [α1, α2, α3], 1/K"),
        num("reference_temperature", "Reference temperature", D::Temperature).def("0"),
    ],
);

pub const MATERIAL: &[Field] = &[
    NAME,
    ELASTIC,
    positive("density", "Density", D::Density),
    PLASTIC,
    HYPERELASTIC,
    DAMAGE,
    EROSION,
    THERMAL,
    variants("fluid", "Fluid", &[variant("newtonian", "Newtonian", &[positive("viscosity", "Viscosity", D::Viscosity).req()])])
        .hint("a fluid needs a density"),
];

pub const MATERIAL_PLASTICITY: &[Field] = &[reference("name", "Material", C::Material).req(), PLASTIC.req().def(r#"{"type": "bilinear"}"#)];
pub const MATERIAL_DAMAGE: &[Field] =
    &[reference("name", "Material", C::Material).req(), DAMAGE.def(r#"{"type": "johnson_cook"}"#).hint("give damage, erosion or both"), EROSION];

const PROFILE: Field = variants(
    "profile",
    "Profile",
    &[
        variant("rectangle", "Rectangle", &[positive("width", "Width", D::Length).req(), positive("height", "Height", D::Length).req()]),
        variant("circle", "Circle", &[positive("radius", "Radius", D::Length).req()]),
        variant("pipe", "Pipe", &[positive("radius", "Radius", D::Length).req(), positive("thickness", "Wall", D::Length).req()]),
        variant(
            "i",
            "I-beam",
            &[
                positive("height", "Height", D::Length).req(),
                positive("width", "Width", D::Length).req(),
                positive("web", "Web", D::Length).req(),
                positive("flange", "Flange", D::Length).req(),
            ],
        ),
        variant(
            "box",
            "Box",
            &[
                positive("width", "Width", D::Length).req(),
                positive("height", "Height", D::Length).req(),
                positive("thickness", "Wall", D::Length).req(),
            ],
        ),
    ],
)
.req()
.def(r#"{"type": "rectangle"}"#);

pub const SECTION: &[Field] = &[
    NAME,
    reference("set", "Region", C::Set).req(),
    reference("material", "Material", C::Material).req(),
    variants(
        "kind",
        "Kind",
        &[
            variant("solid", "Solid", &[]),
            variant(
                "shell",
                "Shell",
                &[
                    positive("thickness", "Thickness", D::Length).req(),
                    range("offset", "Offset", D::None, -0.5, 0.5).def("0").hint("fraction of the thickness"),
                    int("points", "Integration points", 1, 15).def("5"),
                ],
            ),
            variant("beam", "Beam", &[PROFILE, xyz("orientation", "Orientation", D::None).req().def("[0, 0, 1]")]),
            variant("rod", "Rod", &[positive("area", "Area", D::Area).req()]),
            variant("plane_stress", "Plane stress", &[positive("thickness", "Thickness", D::Length).req().def("1")]),
            variant("plane_strain", "Plane strain", &[positive("thickness", "Thickness", D::Length).def("1")]),
            variant("axisymmetric", "Axisymmetric", &[]),
        ],
    )
    .req()
    .def(r#"{"type": "solid"}"#),
    group(
        "formulation",
        "Formulation",
        &[
            choice("integration", "Integration", &[("full", "Full"), ("reduced", "Reduced")]).def("\"full\""),
            flag("incompatible_modes", "Incompatible modes").def("false"),
            flag("hybrid", "Hybrid").def("false"),
        ],
    ),
];

pub const REFERENCE_POINT: &[Field] = &[
    NAME,
    xyz("xyz", "Position", D::Length).req().def("[0, 0, 0]"),
    int("label", "Node label", 0, u32::MAX as i64).hint("0: the highest free label"),
];

const DOFS: Field = json("dofs", "Degrees of freedom").hint("[1, …, 6], default all");

pub const COUPLING: &[Field] = &[
    NAME,
    reference("reference_point", "Reference point", C::ReferencePoint).req(),
    reference("set", "Region", C::Set).req(),
    choice("kind", "Kind", &[("kinematic", "Kinematic"), ("distributing", "Distributing")]).req().def("\"kinematic\""),
    DOFS,
];

pub const MASS: &[Field] = &[
    NAME,
    TARGET,
    positive("mass", "Mass", D::Mass).hint("per node; [mx, my, mz] in JSON"),
    vector("inertia", "Inertia", D::Inertia, &["I11", "I22", "I33", "I12", "I13", "I23"]).hint("about the node; needs a reference point"),
];

const SPRING_FIRST: Field = target("first", "First end").req();
const SPRING_SECOND: Field = target("second", "Second end").hint("ground when empty");
const SPRING_DIRECTION: Field =
    either("direction", "Direction", &[int("dof", "DOF", 1, 6), xyz("vector", "Vector", D::None), unit("line", "Along the ends")])
        .req()
        .def(r#"{"dof": 1}"#);

pub const SPRING: &[Field] = &[
    NAME,
    variants(
        "",
        "Type",
        &[
            variant("spring", "Spring", &[SPRING_FIRST, SPRING_SECOND, SPRING_DIRECTION, positive("stiffness", "Stiffness", D::Stiffness).req()]),
            variant("dashpot", "Dashpot", &[SPRING_FIRST, SPRING_SECOND, SPRING_DIRECTION, positive("coefficient", "Coefficient", D::Damping).req()]),
        ],
    )
    .req()
    .def("\"spring\""),
];

/// The target of a boundary condition, force, moment or temperature.
const TARGET: Field = target("target", "Target").req();

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
                        .req()
                        .def("[0, 0, 0, null, null, null]")
                        .hint("empty: free"),
                ],
            ),
            variant("symmetry", "Symmetry", &[TARGET, choice("plane", "Plane normal", &[("x", "X"), ("y", "Y"), ("z", "Z")]).req().def("\"x\"")]),
            variant("temperature", "Temperature", &[TARGET, num("value", "Temperature", D::Temperature).req()]),
        ],
    )
    .req()
    .def("\"fixed\""),
];

const DISTRIBUTION: Field = choice("distribution", "Distribution", &[("total", "Total"), ("per_node", "Per node")]).def("\"total\"");
const LOAD_SET: Field = reference("set", "Region", C::Set).req();

pub const LOAD: &[Field] = &[
    NAME,
    variants(
        "",
        "Type",
        &[
            variant("force", "Force", &[TARGET, xyz("vector", "Force", D::Force).req(), DISTRIBUTION]),
            variant("moment", "Moment", &[TARGET, xyz("vector", "Moment", D::Moment).req(), DISTRIBUTION]),
            variant("pressure", "Pressure", &[LOAD_SET, num("magnitude", "Pressure", D::Stress).req().hint("positive pushes in")]),
            variant("traction", "Traction", &[LOAD_SET, xyz("vector", "Traction", D::Stress).req()]),
            variant(
                "gravity",
                "Gravity",
                &[xyz("acceleration", "Acceleration", D::Acceleration).req(), reference("set", "Region", C::Set).hint("the whole model")],
            ),
            variant("body_force", "Body force", &[LOAD_SET, xyz("vector", "Force per volume", D::ForcePerVolume).req()]),
            variant(
                "centrifugal",
                "Centrifugal",
                &[
                    num("angular_velocity", "Angular velocity", D::AngularVelocity).req(),
                    xyz("point", "Point on axis", D::Length).req().def("[0, 0, 0]"),
                    xyz("axis", "Axis", D::None).req().def("[0, 0, 1]"),
                    reference("set", "Region", C::Set).hint("the whole model"),
                ],
            ),
            variant("surface_flux", "Surface heat flux", &[LOAD_SET, num("flux", "Flux", D::HeatFlux).req()]),
            variant(
                "convection",
                "Convection",
                &[LOAD_SET, positive("film", "Film coefficient", D::FilmCoefficient).req(), num("sink", "Sink temperature", D::Temperature).req()],
            ),
            variant("heat_source", "Heat source", &[LOAD_SET, num("power", "Power per volume", D::None).req()]),
            variant("temperature", "Temperature", &[TARGET, num("value", "Temperature", D::Temperature).req()]),
        ],
    )
    .req()
    .def("\"force\""),
];

const FACE_CONDITION: &[Variant] = &[
    variant("wall", "Wall", &[]),
    variant("moving_wall", "Moving wall", &[xyz("velocity", "Velocity", D::Velocity).req()]),
    variant("slip", "Slip", &[]),
    variant(
        "velocity",
        "Velocity inlet",
        &[
            xyz("velocity", "Velocity", D::Velocity).req(),
            choice("profile", "Profile", &[("uniform", "Uniform"), ("parabolic", "Parabolic")]).def("\"uniform\""),
        ],
    ),
    variant("pressure", "Pressure outlet", &[num("pressure", "Pressure", D::Stress).req().def("0")]),
    variant("periodic", "Periodic", &[]),
];

const fn face(key: &'static str, label: &'static str) -> Field {
    variants(key, label, FACE_CONDITION).hint("wall")
}

pub const FLUID_DOMAIN: &[Field] = &[
    NAME,
    reference("material", "Fluid", C::Material).req(),
    xyz("min", "Box min", D::Length).req(),
    xyz("max", "Box max", D::Length).req(),
    positive("spacing", "Cell size", D::Length).req(),
    group(
        "faces",
        "Box faces",
        &[
            face("x_min", "X min"),
            face("x_max", "X max"),
            face("y_min", "Y min"),
            face("y_max", "Y max"),
            face("z_min", "Z min"),
            face("z_max", "Z max"),
        ],
    ),
    references("obstacles", "Obstacles", C::Body),
    json("surfaces", "Obstacle faces").hint("[{\"face\", \"condition\"}]; walls by default"),
    xyz("initial_velocity", "Initial velocity", D::Velocity),
    xyz("acceleration", "Acceleration", D::Acceleration),
    positive("tolerance", "Obstacle tolerance", D::Length).hint("a tenth of the cell size"),
];

pub const FLUID_BOUNDARY: &[Field] = &[
    reference("domain", "Domain", C::FluidDomain).req(),
    json("at", "Boundary").req().def("\"x_min\"").hint("\"x_min\" … \"z_max\", or {\"body\", \"face\"}"),
    variants("condition", "Condition", FACE_CONDITION).req().def(r#"{"type": "velocity"}"#),
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
                flag("nonlinear_geometry", "Nonlinear geometry").def("true"),
                range("initial_increment", "Initial increment", D::None, 0.0, 1.0).def("0.1"),
                range("min_increment", "Min increment", D::None, 0.0, 1.0).def("1e-5"),
                range("max_increment", "Max increment", D::None, 0.0, 1.0).def("1.0"),
                int("max_increments", "Max increments", 1, 1_000_000).def("100"),
                int("max_iterations", "Iterations per increment", 1, 1000).def("16"),
                group(
                    "tolerances",
                    "Tolerances",
                    &[
                        positive("force", "Force", D::None).def("1e-6"),
                        positive("displacement", "Displacement", D::None).def("1e-5"),
                        positive("energy", "Energy", D::None).def("1e-10"),
                    ],
                ),
                flag("line_search", "Line search").def("false"),
                json("load_path", "Load path").hint("[[fraction, factor], …]; a ramp when empty"),
                group(
                    "imperfection",
                    "Imperfection",
                    &[
                        reference("step", "Buckling step", C::Step).req(),
                        int("mode", "Mode", 1, 100_000).req().def("1"),
                        num("scale", "Largest component", D::Length).req(),
                    ],
                ),
                group(
                    "arc_length",
                    "Arc length",
                    &[
                        choice("constraint", "Constraint", &[("cylindrical", "Cylindrical"), ("spherical", "Spherical")]).def("\"cylindrical\""),
                        positive("max_load_factor", "Max load factor", D::None).def("1"),
                        json("displacement_limit", "Displacement limit").hint("{\"node\": 1, \"direction\": 3, \"displacement\": -10}"),
                    ],
                ),
            ],
        ),
        variant("buckle", "Buckling", &[int("modes", "Modes", 1, 100_000).req().def("4"), num("shift", "Shift", D::None)]),
        variant(
            "frequency",
            "Frequency",
            &[
                int("modes", "Modes", 1, 10_000).req().def("10"),
                positive("min_hz", "Min frequency", D::Hertz),
                positive("max_hz", "Max frequency", D::Hertz),
            ],
        ),
        variant(
            "explicit_dynamic",
            "Dynamic, explicit",
            &[
                positive("end_time", "End time", D::Time).req(),
                range("dt_scale", "Time step scale", D::None, 0.0, 1.0).def("0.9"),
                int("max_increments", "Max increments", 1, 10_000_000_000).def("10000000"),
                positive("output_interval", "Field output every", D::Time),
                positive("history_interval", "History every", D::Time),
                reference("history_nodes", "History nodes", C::Set),
                rows(
                    "initial_velocities",
                    "Initial velocities",
                    &[reference("set", "Set", C::Set).req(), xyz("velocity", "Velocity", D::Velocity).req()],
                ),
                rows(
                    "rigid_walls",
                    "Rigid walls",
                    &[
                        reference("set", "Set", C::Set).req(),
                        xyz("point", "Point", D::Length).req(),
                        xyz("normal", "Normal", D::None).req(),
                        FRICTION,
                    ],
                ),
                group(
                    "hourglass",
                    "Hourglass control",
                    &[
                        choice("type", "Type", &[("viscous", "Viscous"), ("stiffness", "Stiffness")]).def("\"viscous\""),
                        positive("coefficient", "Coefficient", D::None),
                    ],
                ),
                group("bulk_viscosity", "Bulk viscosity", &[positive("linear", "Linear", D::None), positive("quadratic", "Quadratic", D::None)]),
                flag("nonlinear_geometry", "Nonlinear geometry").def("true"),
                choice("stress_rate", "Stress rate", &[("jaumann", "Jaumann"), ("green_naghdi", "Green–Naghdi")]),
                group(
                    "mass_scaling",
                    "Mass scaling",
                    &[positive("target_increment", "Target increment", D::Time).req(), reference("set", "Set", C::Set)],
                ),
            ],
        ),
        variant(
            "heat_transfer",
            "Heat transfer",
            &[group(
                "transient",
                "Transient",
                &[
                    positive("end_time", "End time", D::Time).req(),
                    positive("increment", "Increment", D::Time).req(),
                    range("theta", "θ", D::None, 0.0, 1.0),
                    num("initial_temperature", "Initial temperature", D::Temperature),
                ],
            )
            .hint("steady state when off")],
        ),
        variant("coupled_temperature_displacement", "Coupled temperature–displacement", &[]),
        variant(
            "fluid",
            "Fluid (lattice Boltzmann)",
            &[
                reference("domain", "Domain", C::FluidDomain).req(),
                positive("end_time", "End time", D::Time).hint("or Steps"),
                int("steps", "Steps", 1, 1_000_000_000_000),
                positive("time_step", "Time step", D::Time).hint("chosen for Mach 0.087"),
                int("frames", "Frames", 1, 100_000).def("10"),
                positive("magic", "TRT Λ", D::None).hint("0.1875"),
            ],
        ),
    ],
);

const OUTPUT: Field = json("output", "Output").hint("[\"displacement\", \"stress\", …]; by procedure when empty");
const STEP_BCS: Field = references("boundary_conditions", "Boundary conditions", C::BoundaryCondition);
const STEP_LOADS: Field = references("loads", "Loads", C::Load);

pub const STEP_CREATE: &[Field] =
    &[NAME, PROCEDURE.req().def(r#"{"type": "static"}"#), OUTPUT, STEP_BCS, STEP_LOADS, reference("before", "Insert before", C::Step).hint("last")];

pub const STEP_EDIT: &[Field] = &[reference("name", "Step", C::Step).req(), PROCEDURE, OUTPUT, STEP_BCS, STEP_LOADS];

pub const STEP_ACTIVATE: &[Field] = &[
    reference("step", "Step", C::Step).req(),
    references("boundary_conditions", "Boundary conditions", C::BoundaryCondition),
    references("loads", "Loads", C::Load),
    flag("active", "Active").def("true"),
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
)
.req()
.def("\"mm-t-s-N-MPa\"")];

// ---- Contact ----

const FRICTION: Field = group(
    "friction",
    "Friction",
    &[
        positive("static", "Static μ", D::None).hint("0"),
        positive("kinetic", "Kinetic μ", D::None).hint("the static μ"),
        positive("decay", "Decay", D::None).hint("0; per slip rate"),
    ],
)
.hint("frictionless");
const PENALTY: Field = range("penalty_scale", "Penalty scale", D::None, 0.0, 1.0).def("1");
const PENETRATION: Field =
    choice("initial_penetration", "Initial penetration", &[("adjust", "Adjust nodes"), ("refuse", "Refuse")]).def("\"adjust\"");

pub const CONTACT_PAIR: &[Field] = &[
    NAME,
    reference("main", "Main surface", C::Set).req(),
    reference("secondary", "Secondary surface", C::Set).req(),
    FRICTION,
    PENALTY,
    PENETRATION,
];
pub const CONTACT_GENERAL: &[Field] = &[NAME, reference("surface", "Surface", C::Set).hint("every exterior face"), FRICTION, PENALTY, PENETRATION];
pub const CONTACT_EDIT: &[Field] = &[
    reference("name", "Contact", C::Contact).req(),
    variants(
        "surfaces",
        "Surfaces",
        &[
            variant("pair", "Pair", &[reference("main", "Main", C::Set).req(), reference("secondary", "Secondary", C::Set).req()]),
            variant("general", "General", &[reference("surface", "Surface", C::Set).hint("every exterior face")]),
        ],
    )
    .req(),
    FRICTION,
    PENALTY,
    PENETRATION,
];
pub const TIE: &[Field] = &[
    NAME,
    reference("main", "Main surface", C::Set).req(),
    reference("secondary", "Secondary surface", C::Set).req(),
    positive("position_tolerance", "Position tolerance", D::Length).hint("half the main element's thickness"),
];
pub const TIE_EDIT: &[Field] = &[
    reference("name", "Tie", C::Tie).req(),
    reference("main", "Main surface", C::Set).req(),
    reference("secondary", "Secondary surface", C::Set).req(),
    positive("position_tolerance", "Position tolerance", D::Length).hint("half the main element's thickness"),
];
pub const CONTACT_FIND: &[Field] = &[
    positive("tolerance", "Tolerance", D::Length).hint("a tenth of the mean face size"),
    flag("create", "Create pairs").def("false"),
    FRICTION,
    PENALTY,
    PENETRATION,
];

// ---- Meshing ----

const BODY: Field = reference("body", "Body", C::Body).hint("every body of the design");
const SURFACE: Field = int("surface", "Imported surface", 0, 1_000_000).hint("instead of a body");
const TOLERANCE: Field = positive("tolerance", "Chordal tolerance", D::Millimetre).hint("1e-3 of the diagonal");
const WELD: Field = range("weld", "Weld", D::None, 0.0, 1e-3).hint("1e-9 of the diagonal");
const WELD_DISTANCE: Field = positive("weld_distance", "Weld distance", D::Millimetre).hint("the surface's own");
const FEATURE_ANGLE: Field = range("feature_angle", "Feature angle", D::Degree, 0.0, 180.0).hint("45");

pub const TET: &[Field] = &[
    BODY,
    SURFACE,
    positive("size", "Element size", D::Millimetre).hint("1/20 of the diagonal"),
    choice("order", "Order", &[("linear", "Linear (tet4)"), ("quadratic", "Quadratic (tet10)")]).def("\"linear\""),
    TOLERANCE,
    flag("quality", "Improve quality").def("true"),
    range("quality_target", "Quality target", D::Degree, 5.0, 60.0).hint("40"),
    range("curvature_angle", "Curvature angle", D::Degree, 0.0, 90.0).hint("22.5"),
    range("grading", "Grading", D::None, 0.05, 2.0).hint("0.3"),
    int("max_passes", "Improvement passes", 0, 200).hint("20"),
    int("max_points", "Max points", 1, 20_000_000).hint("1000000"),
    int("max_steiner", "Max Steiner points", 0, 10_000_000).hint("50000"),
    int("max_tets", "Max tetrahedra", 1, 50_000_000).hint("5000000"),
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

const HEX_SIZE: Field = positive("size", "Element size", D::Millimetre).hint("1/25 of the diagonal");
const HEX_ORDER: Field = int("order", "Order", 1, 2).def("1").hint("1: hex8, 2: hex20");

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
    .def("\"auto\""),
    TOLERANCE,
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

pub const HEX_SWEEP: &[Field] = &[
    BODY,
    SURFACE,
    HEX_SIZE,
    HEX_ORDER,
    choice("method", "Method", &[("swept", "Any sweep"), ("extrude", "Extrude"), ("sweep", "Sweep"), ("revolve", "Revolve")]).def("\"swept\""),
    TOLERANCE,
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

pub const HEX_MAPPED: &[Field] = &[BODY, SURFACE, HEX_SIZE, HEX_ORDER, TOLERANCE, WELD, WELD_DISTANCE, FEATURE_ANGLE];

pub const HEX_BLOCKS: &[Field] =
    &[references("bodies", "Cells", C::Body).hint("every body of the design"), HEX_SIZE, HEX_ORDER, TOLERANCE, WELD, WELD_DISTANCE, FEATURE_ANGLE];

pub const CELLS: &[Field] = &[
    json("methods", "Methods").hint(r#"{"cell": "auto | hex | swept | extrude | sweep | revolve | mapped | tet"}"#),
    choice("default", "Other cells", &[("auto", "Auto (hex, else tet)"), ("hex", "Hex"), ("swept", "Swept"), ("mapped", "Mapped"), ("tet", "Tet")])
        .def("\"auto\""),
    references("bodies", "Cells", C::Body).hint("every body of the design"),
    HEX_SIZE,
    HEX_ORDER,
    TOLERANCE,
];

pub const MIRROR: &[Field] = &[
    json("plane", "Plane").req().hint(r#""Datum" or {"point": [x, y, z], "normal": [nx, ny, nz]}"#),
    references("bodies", "Bodies", C::Body).hint("every meshed body"),
    json("as", "Images").hint(r#"{"body": "image body"}; default: the design's body at the reflected place"#),
];

pub const SYMMETRY: &[Field] = &[
    NAME,
    reference("datum", "Datum plane", C::DatumPlane).req(),
    reference("set", "Set", C::Set).hint("the faces of the design on the plane"),
    references("bodies", "Bodies", C::Body).hint("every body"),
];

pub const HEX_OCTREE: &[Field] = &[
    BODY,
    SURFACE,
    HEX_SIZE,
    TOLERANCE,
    int("max_depth", "Refinement levels", 0, 5).hint("3"),
    int("max_elements", "Max elements", 1, 10_000_000),
    int("attempts", "Attempts", 1, 8).hint("4"),
    int("max_sweeps", "Optimiser sweeps", 0, 10_000).hint("300"),
    WELD,
    WELD_DISTANCE,
    FEATURE_ANGLE,
];

pub const BLOCK: &[Field] = &[
    text("body", "Body").req().def("\"Block\""),
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
    .req()
    .def("\"hex8\""),
    json("corners", "Corners").req().hint("[[x, y, z] × 8] (2D: × 4 at z = 0)"),
    json("divisions", "Divisions").req().hint("[n12, n14, n15] (2D: [n12, n14])"),
    json("bias", "Bias").hint("[1, 1, 1]"),
    json("edges", "Curved and named edges"),
    json("faces", "Face names").hint("{\"1\": \"bottom\"}"),
    json("vertices", "Corner names"),
    text("face", "2D face name").hint("area"),
    positive("merge", "Merge distance", D::Length).hint("1e-6 of the diagonal"),
];

pub const PLANAR: &[Field] = &[
    positive("size", "Element size", D::Length).req(),
    choice("kind", "Element", &[("quad4", "quad4"), ("quad8", "quad8"), ("tri3", "tri3"), ("tri6", "tri6")]).def("\"quad4\""),
    BODY,
    text("face", "Face").hint("a planar face of the body"),
    TOLERANCE,
    json("polygon", "Polygon").hint("[[x, y], …] instead of a face"),
    json("holes", "Holes"),
    text("name", "Body name").hint("Plane"),
    int("max_elements", "Max elements", 1, 4_000_000),
];

pub const QUALITY: &[Field] = &[json("elements", "Elements").hint("[labels]; all when empty"), int("bins", "Histogram bins", 1, 1000).def("10")];

pub const SURFACE_CHECK: &[Field] =
    &[reference("body", "Body", C::Body).req().hint("or an imported surface"), SURFACE, TOLERANCE, WELD, WELD_DISTANCE, FEATURE_ANGLE];

pub const NODE_ADD: &[Field] = &[int("label", "Label", 1, u32::MAX as i64).req(), xyz("xyz", "Position", D::Length).req().def("[0, 0, 0]")];
pub const LABEL: &[Field] = &[int("label", "Label", 1, u32::MAX as i64).req()];
pub const ELEMENT_ADD: &[Field] = &[
    int("label", "Label", 1, u32::MAX as i64).req(),
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
    .req()
    .def("\"hex8\""),
    json("nodes", "Nodes").req().hint("[labels]"),
];

pub const SEED_EDGE: &[Field] = &[
    json("edges", "Edges").req().hint("[{\"body\", \"edge\"}, …] — Use selection with Select: Edges"),
    int("count", "Count", 1, 100_000).hint("or an element size"),
    positive("size", "Element size", D::Length).hint("or a count"),
    group(
        "bias",
        "Bias",
        &[
            range("ratio", "Ratio", D::None, 1.0, 1e4).req().def("1"),
            choice("toward", "Small toward", &[("start", "Start"), ("end", "End"), ("both", "Both ends")]).req().def("\"end\""),
        ],
    ),
    choice("constraint", "Constraint", &[("exact", "Exact"), ("min", "At least"), ("free", "Free (a goal)")]).def("\"exact\""),
];

pub const SEED_CLEAR: &[Field] = &[json("edges", "Edges").hint("every seed")];

pub const DELAUNAY_POINTS: &[Field] = &[json("points", "Points").req().hint("[[x, y, z], …]")];

pub const TAG: &[Field] = &[
    group("geometry", "Geometry", &[reference("body", "Body", C::Body).req(), text("face", "Face"), text("edge", "Edge"), text("vertex", "Vertex")])
        .req(),
    json("elements", "Elements"),
    json("faces", "Element faces"),
    json("edges", "Element edges"),
    json("nodes", "Nodes"),
];

// ---- Geometry and files ----

pub const OPEN_DESIGN: &[Field] = &[
    path("path", "Design", false).req(),
    choice("model", "Model", &[("keep", "Keep the current model"), ("new", "Start a new model")]).def("\"keep\""),
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
                    choice("plane", "Plane", &[("yz", "YZ (normal X)"), ("zx", "ZX (normal Y)"), ("xy", "XY (normal Z)")]).req(),
                    num("offset", "Offset", POINT).def("0"),
                ],
            ),
            variant(
                "three_points",
                "Three points",
                &[xyz("a", "Point A", POINT).req(), xyz("b", "Point B", POINT).req(), xyz("c", "Point C", POINT).req()],
            ),
            variant("point_normal", "Point and normal", &[xyz("point", "Point", POINT).req(), xyz("normal", "Normal", D::None).req()]),
        ],
    )
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
                &[choice("axis", "Axis", &[("x", "X"), ("y", "Y"), ("z", "Z")]).req(), xyz("through", "Through", POINT).hint("the origin")],
            ),
            variant("two_points", "Two points", &[xyz("a", "Point A", POINT).req(), xyz("b", "Point B", POINT).req()]),
            variant("point_direction", "Point and direction", &[xyz("point", "Point", POINT).req(), xyz("direction", "Direction", D::None).req()]),
        ],
    )
    .req()
    .def(r#"{"type": "principal"}"#),
];
pub const DATUM_DELETE: &[Field] = existing!("Datum", C::Datum);
pub const PARTITION: &[Field] =
    &[NAME, reference("datum", "Datum plane", C::DatumPlane).req(), references("bodies", "Bodies", C::Body).hint("every body the plane divides")];
pub const PARTITION_DELETE: &[Field] = existing!("Partition", C::Partition);
pub const TESSELLATE: &[Field] = &[reference("body", "Body", C::Body).req(), TOLERANCE];
pub const IMPORT_MESH: &[Field] = &[path("path", "3MF file", false).req()];
pub const FILE_OPEN: &[Field] = &[path("path", "Model", false).req()];
pub const FILE_SAVE: &[Field] = &[path("path", "File", true).hint("the model's file")];
pub const FILE_SAVE_AS: &[Field] = &[path("path", "File", true).req()];
pub const RECOVERY_LIST: &[Field] = &[path("dir", "Folder", false).hint("the recovery folder")];
pub const RECOVER: &[Field] = &[text("id", "Autosave").req(), path("dir", "Folder", false).hint("the recovery folder")];
pub const RECOVERY_DISCARD: &[Field] = &[text("id", "Autosave"), flag("all", "All"), path("dir", "Folder", false).hint("the recovery folder")];
pub const SAMPLE: &[Field] = &[choice(
    "name",
    "Sample",
    &[
        ("cantilever", "Cantilever"),
        ("impact", "Impact"),
        ("parts", "Parts"),
        ("drop-test", "Drop test"),
        ("rubber-seal", "Rubber seal"),
        ("flow-past", "Flow past a bracket"),
        ("heated-bracket", "Heated bracket"),
        ("bracket-hex", "Hex-meshed bracket"),
        ("crank-arm", "Bicycle crank arm"),
        ("crank-mirror", "Mirrored crank arm"),
    ],
)
.req()
.def("\"cantilever\"")];
pub const COUNT: &[Field] = &[int("count", "Count", 1, 1_000_000).def("1")];

pub const DECK: &[Field] = &[
    choice("format", "Format", &[("abaqus", "Abaqus input (.inp)"), ("nastran", "Nastran bulk data (.bdf)"), ("lsdyna", "LS-DYNA keyword (.k)")])
        .req()
        .def("\"abaqus\""),
    path("path", "File", true).req(),
    references("steps", "Steps", C::Step).hint("all"),
];

// ---- Solving ----

const STEP: Field = reference("step", "Step", C::Step).hint("the only one");
const SOLVED: Field = reference("step", "Step", C::SolvedStep).hint("the step shown");
const MAX_DOFS: Field = int("max_dofs", "Max DOFs", 1, 6_000_000).hint("6000000");
const MAX_FACTOR: Field = int("max_factor_entries", "Max factor entries", 1, 400_000_000).hint("400000000");
const THREADS: Field = int("threads", "Threads", 1, 256).hint("the session's");

pub const SOLVE_STATIC: &[Field] = &[STEP, MAX_DOFS, MAX_FACTOR, THREADS];
pub const SOLVE_NONLINEAR: &[Field] =
    &[STEP, MAX_DOFS, MAX_FACTOR, int("max_newton_iterations", "Max Newton iterations", 1, 100_000).hint("100000"), THREADS];
pub const SOLVE_FREQUENCY: &[Field] = &[
    STEP,
    choice("mass", "Mass matrix", &[("consistent", "Consistent"), ("lumped", "Lumped")]).def("\"consistent\""),
    num("shift", "Shift", D::None).hint("0"),
    MAX_DOFS,
    MAX_FACTOR,
    int("max_iterations", "Max Lanczos vectors", 1, 3000).hint("3000"),
    THREADS,
];
pub const SOLVE_BUCKLE: &[Field] = &[STEP, MAX_DOFS, MAX_FACTOR, int("max_iterations", "Max Lanczos vectors", 1, 3000).hint("3000"), THREADS];
pub const SOLVE_EXPLICIT: &[Field] = &[
    STEP,
    MAX_DOFS,
    int("max_increments", "Max increments", 1, 1_000_000_000).hint("1000000000"),
    int("max_output_values", "Max output values", 1, 50_000_000).hint("50000000"),
    int("max_contact_faces", "Max contact faces", 1, 10_000_000).hint("10000000"),
    THREADS,
];
pub const SOLVE_HISTORY: &[Field] = &[SOLVED.req(), int("every", "Keep one in", 1, 1_000_000_000)];
pub const SOLVE_FLUID: &[Field] = &[
    STEP,
    int("max_cells", "Max cells", 1, 16_000_000),
    int("max_steps", "Max steps", 1, 10_000_000),
    positive("max_cell_updates", "Max cell updates", D::None),
    positive("max_frame_bytes", "Max frame bytes", D::None),
    THREADS,
];
pub const SOLVE_PROBE: &[Field] = &[
    SOLVED.req(),
    json("nodes", "Nodes").hint("[labels]"),
    reference("set", "Set", C::Set),
    json("at", "Points").hint("[[x, y, z], …]: nearest nodes"),
    int("mode", "Mode", 1, 1_000_000),
    int("frame", "Frame", 0, 1_000_000_000),
    flag("elements", "Element results"),
    json("points", "Fluid points").hint("[[x, y, z], …]"),
    json("forces", "Fluid walls").hint("[\"x_min\", \"F2:top\"]"),
];

// ---- Results ----

pub const DEFORMED: &[Field] = &[SOLVED, flag("show", "Show").def("true"), json("scale", "Scale").def("\"auto\"").hint("\"auto\" or a factor")];
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
    .req()
    .def("\"von_mises\""),
    int("bands", "Bands", 2, 24).def("12"),
];
pub const MODES: &[Field] = &[SOLVED, int("mode", "Mode", 1, 1_000_000).def("1")];
pub const ANIMATE: &[Field] = &[flag("on", "Animate").def("true")];
pub const FRAME: &[Field] = &[SOLVED, int("frame", "Frame", 0, 1_000_000_000), positive("time", "Time", D::Time).hint("the last frame")];
pub const REACTIONS: &[Field] = &[SOLVED, references("sets", "Sets", C::Set).hint("the step's BC sets"), xyz("about", "About point", D::Length)];

#[cfg(test)]
mod tests {
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
                let missing: Vec<String> = problems(form, &p).into_iter().filter(|p| p.message == "required").map(|p| p.path).collect();
                match s.execute(c.id, &p) {
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

    const STATE: &[&str] = &["open a design first", "`boundary_conditions` or `loads`", "give one of", "is not an explicit", "has a result"];

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
