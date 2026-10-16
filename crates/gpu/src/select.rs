//! Which adapter draws a headless picture. `FIELDCRAFT_GPU` chooses the policy:
//!
//! - `auto` (the default) walks a hardware GPU, then a software adapter (lavapipe, llvmpipe,
//!   WARP, SwiftShader), then the CPU rasterizer;
//! - `hardware` and `software` are strict: an error when there is no such adapter;
//! - `cpu` forces the CPU rasterizer.
//!
//! Backends are Metal on macOS, DX12 and Vulkan on Windows, Vulkan and GL elsewhere;
//! `WGPU_BACKEND` narrows them. A discrete GPU outranks an integrated one, then a virtual one,
//! then any other; a CPU device is the software tier.

use crate::{GpuError, Result};

/// The adapter policy (`FIELDCRAFT_GPU`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    #[default]
    Auto,
    Hardware,
    Software,
    Cpu,
}

/// The environment variable that chooses the [`Policy`].
pub const ENV: &str = "FIELDCRAFT_GPU";

impl Policy {
    pub fn parse(s: &str) -> Option<Policy> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Policy::Auto,
            "hardware" => Policy::Hardware,
            "software" => Policy::Software,
            "cpu" => Policy::Cpu,
            _ => return None,
        })
    }

    /// The policy `FIELDCRAFT_GPU` names; [`Policy::Auto`] when it is unset.
    pub fn from_env() -> Result<Policy> {
        match std::env::var(ENV) {
            Ok(v) => Policy::parse(&v).ok_or_else(|| GpuError::Policy(v.chars().take(40).collect())),
            Err(_) => Ok(Policy::Auto),
        }
    }
}

/// The kind of device a picture was drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Hardware,
    Software,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Hardware => "hardware GPU",
            Tier::Software => "software GPU",
        }
    }
}

/// An adapter's tier and its rank within it (lower is better).
pub fn rank(t: wgpu::DeviceType) -> (Tier, u8) {
    match t {
        wgpu::DeviceType::DiscreteGpu => (Tier::Hardware, 0),
        wgpu::DeviceType::IntegratedGpu => (Tier::Hardware, 1),
        wgpu::DeviceType::VirtualGpu => (Tier::Hardware, 2),
        wgpu::DeviceType::Other => (Tier::Hardware, 3),
        wgpu::DeviceType::Cpu => (Tier::Software, 0),
    }
}

/// The index of the adapter `policy` picks among adapters of `types`: `Ok(None)` for the CPU
/// rasterizer, an error when a strict policy finds no adapter of its tier.
pub fn pick(types: &[wgpu::DeviceType], policy: Policy) -> Result<Option<usize>> {
    let best = |tier: Tier| types.iter().enumerate().filter(|(_, t)| rank(**t).0 == tier).min_by_key(|(_, t)| rank(**t).1).map(|(i, _)| i);
    match policy {
        Policy::Cpu => Ok(None),
        Policy::Auto => Ok(best(Tier::Hardware).or_else(|| best(Tier::Software))),
        Policy::Hardware => best(Tier::Hardware).map(Some).ok_or_else(|| GpuError::NoAdapter(format!("{ENV}=hardware: no hardware GPU adapter"))),
        Policy::Software => best(Tier::Software)
            .map(Some)
            .ok_or_else(|| GpuError::NoAdapter(format!("{ENV}=software: no software GPU adapter (lavapipe, llvmpipe, WARP)"))),
    }
}

/// The backends tried on this platform, narrowed by `WGPU_BACKEND`.
pub fn backends() -> wgpu::Backends {
    let platform = if cfg!(target_os = "macos") || cfg!(target_os = "ios") {
        wgpu::Backends::METAL
    } else if cfg!(windows) {
        wgpu::Backends::DX12 | wgpu::Backends::VULKAN
    } else {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    };
    match wgpu::Backends::from_env() {
        Some(env) => platform & env,
        None => platform,
    }
}

/// The adapter `policy` picks on this machine and its tier; `Ok(None)` for the CPU rasterizer.
pub fn adapter(policy: Policy) -> Result<Option<(wgpu::Adapter, Tier)>> {
    if policy == Policy::Cpu {
        return Ok(None);
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: backends(), ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let mut adapters = pollster::block_on(instance.enumerate_adapters(backends()));
    let types: Vec<_> = adapters.iter().map(|a| a.get_info().device_type).collect();
    let Some(i) = pick(&types, policy)? else { return Ok(None) };
    let tier = types.get(i).map_or(Tier::Hardware, |t| rank(*t).0);
    Ok((i < adapters.len()).then(|| (adapters.swap_remove(i), tier)))
}

#[cfg(test)]
mod tests {
    use wgpu::DeviceType::*;

    use super::*;

    #[test]
    fn policies_walk_the_tiers() {
        assert_eq!(pick(&[Cpu, IntegratedGpu, DiscreteGpu, VirtualGpu], Policy::Auto), Ok(Some(2)));
        assert_eq!(pick(&[Cpu, Other, IntegratedGpu], Policy::Auto), Ok(Some(2)));
        assert_eq!(pick(&[Cpu, DiscreteGpu], Policy::Software), Ok(Some(0)));
        // Auto falls to a software adapter, then to the CPU rasterizer.
        assert_eq!(pick(&[Cpu], Policy::Auto), Ok(Some(0)));
        assert_eq!(pick(&[], Policy::Auto), Ok(None));
        assert_eq!(pick(&[DiscreteGpu], Policy::Cpu), Ok(None));
        // The strict policies refuse.
        assert!(pick(&[Cpu], Policy::Hardware).is_err());
        assert!(pick(&[DiscreteGpu], Policy::Software).is_err());
        assert!(pick(&[], Policy::Hardware).is_err());
    }

    #[test]
    fn policy_names() {
        assert_eq!(Policy::parse("Software"), Some(Policy::Software));
        assert_eq!(Policy::parse(""), Some(Policy::Auto));
        assert_eq!(Policy::parse("cpu"), Some(Policy::Cpu));
        assert_eq!(Policy::parse("gpu"), None);
    }
}
