//! This module contains the implementation of the `cpuset` cgroup subsystem.
//! 
//! See the Kernel's documentation for more information about this subsystem, found at:
//!  [Documentation/cgroup-v1/cpusets.txt](https://www.kernel.org/doc/Documentation/cgroup-v1/cpusets.txt)
use std::path::PathBuf;
use std::io::{Read, Write};
use std::fs::File;

use {CgroupError, CpuResources, Resources, Controller, ControllIdentifier, Subsystem, Controllers};
use CgroupError::*;

/// A controller that allows controlling the `cpuset` subsystem of a Cgroup.
/// 
/// In essence, this controller is responsible for restricting the tasks in the control group to a
/// set of CPUs and/or memory nodes.
#[derive(Debug, Clone)]
pub struct CpuSetController {
    base: PathBuf,
    path: PathBuf,
}

/// The current state of the `cpuset` controller for this control group.
pub struct CpuSet {
    /// If true, no other control groups can share the CPUs listed in the `cpus` field.
    pub cpu_exclusive: bool,
    /// The list of CPUs the tasks of the control group can run on. This is a comma-separated list
    /// with dashes between numbers representing ranges.
    pub cpus: String,
    /// The list of CPUs that the tasks can effectively run on. This removes the list of CPUs that
    /// the parent (and all of its parents) cannot run on from the `cpus` field of this control
    /// group.
    pub effective_cpus: String,
    /// The list of memory nodes that the tasks can effectively use. This removes the list of nodes that
    /// the parent (and all of its parents) cannot use from the `mems` field of this control
    /// group.
    pub effective_mems: String,
    /// If true, no other control groups can share the memory nodes listed in the `mems` field.
    pub mem_exclusive: bool,
    /// If true, the control group is 'hardwalled'. Kernel memory allocations (except for a few
    /// minor exceptions) are made from the memory nodes designated in the `mems` field.
    pub mem_hardwall: bool,
    /// If true, whenever `mems` is changed via `set_mems()`, the memory stored on the previous
    /// nodes are migrated to the new nodes selected by the new `mems`.
    pub memory_migrate: bool,
    /// Running average of the memory pressured faced by the tasks in the control group.
    pub memory_pressure: u64,
    /// This field is only at the root control group and controls whether the kernel will compute
    /// the memory pressure for control groups or not.
    pub memory_pressure_enabled: Option<bool>,
    /// If true, filesystem buffers are spread across evenly between the nodes specified in `mems`.
    pub memory_spread_page: bool, 
    /// If true, kernel slab caches for file I/O are spread across evenly between the nodes
    /// specified in `mems`.
    pub memory_spread_slab: bool, 
    /// The list of memory nodes the tasks of the control group can use. This is a comma-separated list
    /// with dashes between numbers representing ranges.
    pub mems: String,
    /// If true, the kernel will attempt to rebalance the load between the CPUs specified in the
    /// `cpus` field of this control group.
    pub sched_load_balance: bool,
    /// Represents how much work the kernel should do to rebalance this cpuset.
    ///
    /// | `sched_load_balance` | Effect |
    /// | -------------------- | ------ |
    /// |          -1          | Use the system default value |
    /// |           0          | Only balance loads periodically |
    /// |           1          | Immediately balance the load across tasks on the same core |
    /// |           2          | Immediately balance the load across cores in the same CPU package |
    /// |           4          | Immediately balance the load across CPUs on the same node |
    /// |           5          | Immediately balance the load between CPUs even if the system is NUMA |
    /// |           6          | Immediately balance the load between all CPUs |
    pub sched_relax_domain_level: u64,

}

impl Controller for CpuSetController {
    fn control_type(self: &Self) -> Controllers { Controllers::CpuSet }
    fn get_path<'a>(self: &'a Self) -> &'a PathBuf { &self.path }
    fn get_path_mut<'a>(self: &'a mut Self) -> &'a mut PathBuf { &mut self.path }
    fn get_base<'a>(self: &'a Self) -> &'a PathBuf { &self.base }

    fn apply(self: &Self, res: &Resources) {
        /* get the resources that apply to this controller */
        let res: &CpuResources = &res.cpu;

        if res.update_values {
            /* apply pid_max */
            let _ = self.set_cpus(&res.cpus);
            let _ = self.set_mems(&res.mems);
        }
    }
}

impl ControllIdentifier for CpuSetController {
    fn controller_type() -> Controllers {
        Controllers::CpuSet
    }
}

impl<'a> From<&'a Subsystem> for &'a CpuSetController {
    fn from(sub: &'a Subsystem) -> &'a CpuSetController {
        unsafe {
            match sub {
                Subsystem::CpuSet(c) => c,
                _ => {
                    assert_eq!(1, 0);
                    ::std::mem::uninitialized()
                },
            }
        }
    }
}

fn read_string_from(mut file: File) -> Result<String, CgroupError> {
    let mut string = String::new();
    match file.read_to_string(&mut string) {
        Ok(_) => Ok(string.trim().to_string()),
        Err(e) => Err(CgroupError::ReadError(e)),
    }
}

fn read_u64_from(mut file: File) -> Result<u64, CgroupError> {
    let mut string = String::new();
    match file.read_to_string(&mut string) {
        Ok(_) => string.trim().parse().map_err(|_| ParseError),
        Err(e) => Err(CgroupError::ReadError(e)),
    }
}

impl CpuSetController {
    /// Contructs a new `CpuSetController` with `oroot` serving as the root of the control group.
    pub fn new(oroot: PathBuf) -> Self {
        let mut root = oroot;
        root.push(Self::controller_type().to_string());
        Self {
            base: root.clone(),
            path: root,
        }
    }

    /// Returns the statistics gathered by the kernel for this control group. See the struct for
    /// more information on what information this entails.
    pub fn cpuset(self: &Self) -> CpuSet {
        CpuSet {
            cpu_exclusive: {
                self.open_path("cpuset.cpu_exclusive", false).and_then(|file| {
                    read_u64_from(file)
                }).map(|x| x == 1).unwrap_or(false)
            },
            cpus: {
                self.open_path("cpuset.cpus", false).and_then(read_string_from).unwrap_or("".to_string())
            },
            effective_cpus: {
                self.open_path("cpuset.effective_cpus", false).and_then(read_string_from).unwrap_or("".to_string())
            },
            effective_mems: {
                self.open_path("cpuset.effective_mems", false).and_then(read_string_from).unwrap_or("".to_string())
            },
            mem_exclusive: {
                self.open_path("cpuset.mem_exclusive", false).and_then(read_u64_from)
                    .map(|x| x == 1).unwrap_or(false)
            },
            mem_hardwall: {
                self.open_path("cpuset.mem_hardwall", false).and_then(read_u64_from)
                    .map(|x| x == 1).unwrap_or(false)
            },
            memory_migrate: {
                self.open_path("cpuset.memory_migrate", false).and_then(read_u64_from)
                    .map(|x| x == 1).unwrap_or(false)
            },
            memory_pressure: {
                self.open_path("cpuset.memory_pressure", false).and_then(read_u64_from).unwrap_or(0)
            },
            memory_pressure_enabled: {
                self.open_path("cpuset.memory_pressure_enabled", false).and_then(read_u64_from)
                    .map(|x| x == 1).ok()
            },
            memory_spread_page: {
                self.open_path("cpuset.memory_spread_page", false).and_then(read_u64_from)
                    .map(|x| x == 1).unwrap_or(false)
            },
            memory_spread_slab: {
                self.open_path("cpuset.memory_spread_slab", false).and_then(read_u64_from)
                    .map(|x| x == 1).unwrap_or(false)
            },
            mems: {
                self.open_path("cpuset.mems", false).and_then(read_string_from).unwrap_or("".to_string())
            },
            sched_load_balance: {
                self.open_path("cpuset.sched_load_balance", false).and_then(read_u64_from)
                    .map(|x| x == 1).unwrap_or(false)
            },
            sched_relax_domain_level: {
                self.open_path("cpuset.sched_relax_domain_level", false).and_then(read_u64_from)
                    .unwrap_or(0)
            },
        }
    }

    /// Control whether the CPUs selected via `set_cpus()` should be exclusive to this control
    /// group or not.
    pub fn set_cpu_exclusive(self: &Self, b: bool) -> Result<(), CgroupError> {
        self.open_path("cpuset.cpu_exclusive", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }

    /// Control whether the memory nodes selected via `set_memss()` should be exclusive to this control
    /// group or not.
    pub fn set_mem_exclusive(self: &Self, b: bool) -> Result<(), CgroupError> {
        self.open_path("cpuset.mem_exclusive", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }

    /// Set the CPUs that the tasks in this control group can run on.
    ///
    /// Syntax is a comma separated list of CPUs, with an additional extension that ranges can
    /// be represented via dashes.
    pub fn set_cpus(self: &Self, cpus: &String) -> Result<(), CgroupError> {
        self.open_path("cpuset.cpus", true).and_then(|mut file| {
            file.write_all(cpus.as_ref()).map_err(CgroupError::WriteError)
        })
    }

    /// Set the memory nodes that the tasks in this control group can use.
    ///
    /// Syntax is the same as with `set_cpus()`.
    pub fn set_mems(self: &Self, mems: &String) -> Result<(), CgroupError> {
        self.open_path("cpuset.mems", true).and_then(|mut file| {
            file.write_all(mems.as_ref()).map_err(CgroupError::WriteError)
        })
    }

    /// Controls whether the control group should be "hardwalled", i.e., whether kernel allocations
    /// should exclusively use the memory nodes set via `set_mems()`.
    ///
    /// Note that some kernel allocations, most notably those that are made in interrupt handlers
    /// may disregard this.
    pub fn set_hardwall(self: &Self, b: bool) -> Result<(), CgroupError> {
        self.open_path("cpuset.mem_hardwall", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }

    /// Controls whether the kernel should attempt to rebalance the load between the CPUs specified in the
    /// `cpus` field of this control group.
    pub fn set_load_balancing(self: &Self, b: bool) -> Result<(), CgroupError> {
        self.open_path("cpuset.sched_load_balance", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }

    /// Contorl how much effort the kernel should invest in rebalacing the control group.
    ///
    /// See @CpuSet 's similar field for more information.
    pub fn set_rebalance_relax_domain_level(self: &Self, i: i64) -> Result<(), CgroupError> {
        self.open_path("cpuset.sched_relax_domain_level", true).and_then(|mut file| {
            file.write_all(i.to_string().as_ref()).map_err(CgroupError::WriteError)
        })
    }

    /// Control whether when using `set_mems()` the existing memory used by the tasks should be
    /// migrated over to the now-selected nodes.
    pub fn set_memory_migration(self: &Self, b: bool) -> Result<(), CgroupError> {
        self.open_path("cpuset.memory_migrate", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }

    /// Control whether filesystem buffers should be evenly split across the nodes selected via
    /// `set_mems()`.
    pub fn set_memory_spread_page(self: &Self, b: bool) -> Result<(), CgroupError> {
        self.open_path("cpuset.memory_spread_page", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }

    /// Control whether the kernel's slab cache for file I/O should be evenly split across the
    /// nodes selected via `set_mems()`.
    pub fn set_memory_spread_slab(self: &Self, b: bool) -> Result<(), CgroupError> {
        self.open_path("cpuset.memory_spread_slab", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }

    /// Control whether the kernel should collect information to calculate memory pressure for
    /// control groups.
    ///
    /// Note: This is a no-operation if the control group referred by `self` is not the root
    /// control group.
    pub fn set_enable_memory_pressure(self: &Self, b: bool) -> Result<(), CgroupError> {
        /* XXX: this file should only be present in the root cpuset cg */
        self.open_path("cpuset.memory_pressure_enabled", true).and_then(|mut file| {
            if b {
                file.write_all(b"1").map_err(CgroupError::WriteError)
            } else {
                file.write_all(b"0").map_err(CgroupError::WriteError)
            }
        })
    }
}
