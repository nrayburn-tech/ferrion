//! Module descriptors, decoded from a `module-info` class's attributes (4.7.25 to 4.7.27, and
//! `ModuleResolution`, which the JVMS doesn't define).

use super::{AccessFlags, Attribute, ClassFile};
use crate::error::{Error, ErrorKind, Result};

/// A module descriptor with owned names, for the platform API and B2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleDescriptor {
    pub name: String,
    pub flags: u16,
    pub version: Option<String>,
    pub requires: Vec<Requires>,
    pub exports: Vec<Exports>,
    pub opens: Vec<Exports>,
    /// Service interfaces, in internal form.
    pub uses: Vec<String>,
    pub provides: Vec<Provides>,
    /// From `ModulePackages`, in internal form, when the class has it.
    pub packages: Option<Vec<String>>,
    /// From `ModuleMainClass`, in internal form.
    pub main_class: Option<String>,
    /// The raw value of `ModuleResolution`, when present.
    pub resolution: Option<u16>,
    /// Attributes other than `Module` that couldn't be decoded, and were left out. JDK 21's
    /// `ct.sym` has a `ModuleMainClass` pointing at a `CONSTANT_Utf8` (in `jdk.httpserver`'s
    /// descriptor for releases 18 to 20), which javac accepts.
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requires {
    pub module: String,
    pub flags: u16,
    pub version: Option<String>,
}

/// An `exports` or `opens` entry. `to` is empty when unqualified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exports {
    /// In internal form (`java/lang`).
    pub package: String,
    pub flags: u16,
    pub to: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provides {
    pub service: String,
    pub with: Vec<String>,
}

impl ModuleDescriptor {
    /// Decodes the descriptor of a `module-info` class. A class without a `Module` attribute is
    /// `Malformed`.
    pub fn from_class(class: &ClassFile) -> Result<ModuleDescriptor> {
        let mut descriptor = None;
        let mut packages = None;
        let mut main_class = None;
        let mut resolution = None;
        let owned = |name: crate::text::Name<'_>| name.to_str().map(|s| s.into_owned());
        let mut problems = Vec::new();
        for info in class.attributes() {
            let decoded = match info.decode() {
                Ok(decoded) => decoded,
                Err(error) if info.name().is_ok_and(|n| !n.eq_str("Module")) => {
                    problems.push(error.to_string());
                    continue;
                }
                Err(error) => return Err(error),
            };
            match decoded {
                Attribute::Module(module) => {
                    let names = |list: &[crate::text::Name<'_>]| {
                        list.iter()
                            .map(|&name| owned(name))
                            .collect::<Result<Vec<_>>>()
                    };
                    let exports = |list: &[super::ModuleExport<'_>]| {
                        list.iter()
                            .map(|e| {
                                Ok(Exports {
                                    package: owned(e.package)?,
                                    flags: e.flags.0,
                                    to: names(&e.to)?,
                                })
                            })
                            .collect::<Result<Vec<_>>>()
                    };
                    descriptor = Some(ModuleDescriptor {
                        name: owned(module.name)?,
                        flags: module.flags.0,
                        version: module.version.map(owned).transpose()?,
                        requires: module
                            .requires
                            .iter()
                            .map(|r| {
                                Ok(Requires {
                                    module: owned(r.module)?,
                                    flags: r.flags.0,
                                    version: r.version.map(owned).transpose()?,
                                })
                            })
                            .collect::<Result<_>>()?,
                        exports: exports(&module.exports)?,
                        opens: exports(&module.opens)?,
                        uses: names(&module.uses)?,
                        provides: module
                            .provides
                            .iter()
                            .map(|p| {
                                Ok(Provides {
                                    service: owned(p.service)?,
                                    with: names(&p.with)?,
                                })
                            })
                            .collect::<Result<_>>()?,
                        packages: None,
                        main_class: None,
                        resolution: None,
                        problems: Vec::new(),
                    });
                }
                Attribute::ModulePackages(list) => {
                    packages = Some(list.map(|name| owned(name?)).collect::<Result<Vec<_>>>()?);
                }
                Attribute::ModuleMainClass(name) => main_class = Some(owned(name)?),
                Attribute::ModuleResolution(value) => resolution = Some(value),
                _ => {}
            }
        }
        let mut descriptor = descriptor.ok_or_else(|| {
            Error::new(
                ErrorKind::Malformed,
                "a module-info class has no Module attribute",
            )
        })?;
        descriptor.packages = packages;
        descriptor.main_class = main_class;
        descriptor.resolution = resolution;
        descriptor.problems = problems;
        Ok(descriptor)
    }

    /// Whether the module is left out of the default root modules: the low bit of
    /// `ModuleResolution` ("do not resolve by default", as observed for incubator modules).
    pub fn is_resolved_by_default(&self) -> bool {
        self.resolution.is_none_or(|value| value & 1 == 0)
    }

    /// The packages exported to every module, in internal form.
    pub fn unqualified_exports(&self) -> impl Iterator<Item = &str> {
        self.exports
            .iter()
            .filter(|e| e.to.is_empty())
            .map(|e| e.package.as_str())
    }

    /// Whether a `requires` is `static` (`ACC_STATIC_PHASE`).
    pub fn is_static(requires: &Requires) -> bool {
        AccessFlags(requires.flags).contains(AccessFlags::STATIC_PHASE)
    }
}
