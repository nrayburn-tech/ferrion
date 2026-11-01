//! Names in the internal form of JVMS 4.2.1: `java/util/Map$Entry`, `java/util`, `java.base`.

use std::fmt;
use std::sync::Arc;

macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Arc<str>);

        impl $name {
            pub fn new(name: &str) -> $name {
                $name(name.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:?})", stringify!($name), &*self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(name: &str) -> $name {
                $name::new(name)
            }
        }
    };
}

name_type!(
    /// A class's binary name in internal form: `java/util/Map$Entry`.
    ClassName
);
name_type!(
    /// A package in internal form: `java/util`; the unnamed package is empty.
    PackageName
);
name_type!(
    /// A module name: `java.base`.
    ModuleName
);

impl ClassName {
    /// The package part: `java/util` for `java/util/Map$Entry`.
    pub fn package(&self) -> &str {
        self.0.rsplit_once('/').map_or("", |(package, _)| package)
    }

    /// The part after the package: `Map$Entry`.
    pub fn simple(&self) -> &str {
        self.0
            .rsplit_once('/')
            .map_or(&self.0, |(_, simple)| simple)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts() {
        let name = ClassName::new("java/util/Map$Entry");
        assert_eq!(name.package(), "java/util");
        assert_eq!(name.simple(), "Map$Entry");
        let name = ClassName::from("Top");
        assert_eq!((name.package(), name.simple()), ("", "Top"));
        assert_eq!(format!("{name:?}"), "ClassName(\"Top\")");
    }
}
