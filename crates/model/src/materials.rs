//! Materials: linear elasticity and mass density.

use serde::{Deserialize, Serialize};

use crate::{Error, Model, Result, insert_new, not_in_use, positive, replace_existing};

/// Linear elastic behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Elastic {
    /// Young's modulus and Poisson's ratio.
    Isotropic { young: f64, poisson: f64 },
    /// Orthotropic, by engineering constants in the material axes 1, 2, 3 (the global X, Y, Z
    /// until material orientations land): Young's moduli `[E1, E2, E3]`, Poisson's ratios
    /// `[ν12, ν13, ν23]` (ν_ij is the contraction in j under stress in i) and shear moduli
    /// `[G12, G13, G23]`.
    EngineeringConstants { young: [f64; 3], poisson: [f64; 3], shear: [f64; 3] },
}

impl Elastic {
    /// Check the constants describe a stable material (positive-definite stiffness).
    pub fn validate(&self) -> Result<()> {
        match *self {
            Elastic::Isotropic { young, poisson } => {
                positive("Young's modulus", young)?;
                if !(poisson > -1.0 && poisson < 0.5) {
                    return Err(Error::Invalid(format!("Poisson's ratio must be above -1 and below 0.5, got {poisson}")));
                }
            }
            Elastic::EngineeringConstants { young, poisson, shear } => {
                for (i, e) in young.iter().enumerate() {
                    positive(&format!("E{}", i + 1), *e)?;
                }
                for (i, g) in shear.iter().enumerate() {
                    positive(["G12", "G13", "G23"].get(i).copied().unwrap_or("G"), *g)?;
                }
                let [e1, e2, e3] = young;
                let [n12, n13, n23] = poisson;
                if poisson.iter().any(|n| !n.is_finite()) {
                    return Err(Error::Invalid("Poisson's ratios must be finite numbers".into()));
                }
                // Reciprocal ratios ν_ji = ν_ij E_j / E_i; stability needs |ν_ij| < sqrt(E_i/E_j)
                // and a positive determinant of the compliance's normal block.
                let (n21, n31, n32) = (n12 * e2 / e1, n13 * e3 / e1, n23 * e3 / e2);
                for (name, nij, ei, ej) in [("ν12", n12, e1, e2), ("ν13", n13, e1, e3), ("ν23", n23, e2, e3)] {
                    if !(nij.abs() < (ei / ej).sqrt()) {
                        return Err(Error::Invalid(format!("{name} = {nij} is too large for the moduli (must be below {})", (ei / ej).sqrt())));
                    }
                }
                let det = 1.0 - n12 * n21 - n23 * n32 - n31 * n13 - 2.0 * n21 * n32 * n13;
                if !(det > 0.0) {
                    return Err(Error::Invalid("the Poisson's ratios make the material unstable (stiffness not positive definite)".into()));
                }
            }
        }
        Ok(())
    }
}

/// A material.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elastic: Option<Elastic>,
    /// Mass per volume, needed by frequency steps and gravity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
}

impl Material {
    pub fn validate(&self) -> Result<()> {
        if self.elastic.is_none() && self.density.is_none() {
            return Err(Error::Invalid("a material needs elastic constants or a density".into()));
        }
        if let Some(e) = &self.elastic {
            e.validate()?;
        }
        if let Some(d) = self.density {
            positive("density", d)?;
        }
        Ok(())
    }
}

impl Model {
    pub fn materials(&self) -> &std::collections::BTreeMap<String, Material> {
        &self.materials
    }

    pub fn material(&self, name: &str) -> Option<&Material> {
        self.materials.get(name)
    }

    pub fn add_material(&mut self, name: &str, material: Material) -> Result<()> {
        material.validate().map_err(|e| context("material", name, e))?;
        insert_new(&mut self.materials, "material", name, material)
    }

    pub fn replace_material(&mut self, name: &str, material: Material) -> Result<()> {
        material.validate().map_err(|e| context("material", name, e))?;
        replace_existing(&mut self.materials, "material", name, material)
    }

    pub fn remove_material(&mut self, name: &str) -> Result<()> {
        let users = self.sections.iter().filter(|(_, s)| s.material == name).map(|(n, _)| format!("section `{n}`")).collect();
        not_in_use("material", name, users)?;
        self.materials.remove(name).map(|_| ()).ok_or_else(|| Error::NotFound(format!("material `{name}`")))
    }
}

/// Prefix an error with the definition it is about.
pub(crate) fn context(what: &str, name: &str, e: Error) -> Error {
    let shown: String = name.chars().take(crate::MAX_NAME_LEN).collect();
    match e {
        Error::Invalid(m) => Error::Invalid(format!("{what} `{shown}`: {m}")),
        Error::NotFound(m) => Error::NotFound(format!("{what} `{shown}`: {m}")),
        Error::Limit(m) => Error::Limit(format!("{what} `{shown}`: {m}")),
        Error::InUse(m) => Error::InUse(format!("{what} `{shown}`: {m}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(young: f64, poisson: f64) -> Material {
        Material { elastic: Some(Elastic::Isotropic { young, poisson }), density: None }
    }

    #[test]
    fn materials_validate_and_name_the_bad_constant() {
        let mut m = Model::default();
        m.add_material("Steel", Material { density: Some(7.85e-9), ..iso(2.1e5, 0.3) }).unwrap();
        assert!(m.add_material("Steel", iso(1.0, 0.3)).is_err(), "duplicate");
        for bad in [iso(0.0, 0.3), iso(-1.0, 0.3), iso(f64::NAN, 0.3), iso(1.0, 0.5), iso(1.0, -1.0), iso(1.0, f64::INFINITY), Material::default()] {
            let e = m.add_material("Bad", bad).unwrap_err();
            assert!(e.to_string().contains("material `Bad`"), "{e}");
        }
        assert!(m.add_material("Light", Material { elastic: None, density: Some(-1.0) }).is_err());
        assert!(m.add_material("1st", iso(1.0, 0.3)).is_err(), "names start with a letter");
        assert!(m.add_material("has space", iso(1.0, 0.3)).is_err());
        m.replace_material("Steel", iso(2.0e5, 0.29)).unwrap();
        assert!(m.replace_material("Nope", iso(2.0e5, 0.29)).is_err());
        assert_eq!(m.materials().len(), 1);
    }

    #[test]
    fn orthotropic_stability() {
        let ortho = |poisson| Elastic::EngineeringConstants { young: [140e3, 10e3, 10e3], poisson, shear: [5e3, 5e3, 3.8e3] };
        ortho([0.3, 0.3, 0.4]).validate().unwrap();
        // ν21 = ν12 E2/E1 is small; ν12 itself may exceed 0.5 in a stiff direction.
        ortho([0.9, 0.3, 0.4]).validate().unwrap();
        assert!(ortho([4.0, 0.3, 0.4]).validate().is_err(), "|ν12| < sqrt(E1/E2) = 3.74");
        assert!(ortho([0.3, 0.3, 1.0]).validate().is_err());
        assert!(ortho([3.5, 3.5, 0.9]).validate().is_err(), "determinant");
        assert!(ortho([0.3, f64::NAN, 0.4]).validate().is_err());
        let zero_shear = Elastic::EngineeringConstants { young: [1.0; 3], poisson: [0.0; 3], shear: [1.0, 0.0, 1.0] };
        assert!(zero_shear.validate().unwrap_err().to_string().contains("G13"));
    }
}
