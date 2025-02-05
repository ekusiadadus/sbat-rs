// Copyright 2023 Google LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! SBAT revocations.
//!
//! Typically this data is read from a UEFI variable. See the crate
//! documentation for details of how it is used.

use crate::csv::{parse_csv, Record};
use crate::metadata::{Entry, Metadata};
use crate::vec::Veclike;
use crate::{Component, Error, Result};
use ascii::AsciiStr;

/// The first entry has the component name and generation like the
/// others, but may also have a date field.
const MAX_HEADER_FIELDS: usize = 3;

/// Whether an image is allowed or revoked.
#[must_use]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationResult<'r, 'a> {
    /// The image has not been revoked.
    Allowed,

    /// The image has been revoked. The first revoked entry is provided
    /// (there could be additional revoked components).
    Revoked(&'r Entry<'a>),
}

/// SBAT revocation data.
///
/// This contains SBAT revocation data parsed from a UEFI variable such
/// as `SbatLevel`.
///
/// See the [crate] documentation for a usage example.
#[derive(Debug, Eq, PartialEq)]
pub struct Revocations<'a, Storage>
where
    Storage: Veclike<Component<'a>>,
{
    date: Option<&'a AsciiStr>,
    components: Storage,
}

impl<'a, Storage> Revocations<'a, Storage>
where
    Storage: Veclike<Component<'a>>,
{
    /// Create a new `Revocations` using `components` for
    /// storage. Existing data in `components` is not cleared. The
    /// `date` is set to `None`.
    pub fn new(components: Storage) -> Self {
        Self {
            components,
            date: None,
        }
    }

    /// Parse SBAT data from raw CSV. This data typically comes from a
    /// UEFI variable. Each record is parsed as a [`Component`].
    ///
    /// Any existing data is cleared before parsing.
    pub fn parse(&mut self, input: &'a [u8]) -> Result<()> {
        self.components.clear();

        let mut first = true;

        parse_csv(input, |record: Record<MAX_HEADER_FIELDS>| {
            if first {
                self.date = record.get_field(2);
                first = false;
            }

            self.components.try_push(Component {
                name: record.get_field(0).ok_or(Error::TooFewFields)?,
                generation: record
                    .get_field_as_generation(1)?
                    .ok_or(Error::TooFewFields)?,
            })
        })
    }

    /// Date when the data was last updated. This is optional metadata
    /// in the first entry and may not be present.
    pub fn date(&self) -> &Option<&AsciiStr> {
        &self.date
    }

    /// Check if the `input` [`Component`] is revoked.
    ///
    /// The `input` is checked against each revocation component. If the
    /// names match, and if the `input`'s version is less than the
    /// version in the corresponding revocation component, the `input`
    /// is considered revoked and the image will not pass validation. If
    /// the `input` is not in the revocation list then it is implicitly
    /// allowed.
    #[must_use]
    pub fn is_component_revoked(&self, input: &Component) -> bool {
        self.components.as_slice().iter().any(|revoked_component| {
            input.name == revoked_component.name
                && input.generation < revoked_component.generation
        })
    }

    /// Check if any component in `metadata` is revoked.
    ///
    /// Each component in the image metadata is checked against the
    /// revocation entries. If the name matches, and if the component's
    /// version is less than the version in the corresponding revocation
    /// entry, the component is considered revoked and the image will
    /// not pass validation. If a component is not in the revocation
    /// list then it is implicitly allowed.
    pub fn validate_metadata<'r, 'b, MetadataStorage>(
        &self,
        metadata: &'r Metadata<'b, MetadataStorage>,
    ) -> ValidationResult<'r, 'b>
    where
        MetadataStorage: Veclike<Entry<'b>>,
    {
        if let Some(revoked_entry) = metadata
            .entries()
            .iter()
            .find(|entry| self.is_component_revoked(&entry.component))
        {
            ValidationResult::Revoked(revoked_entry)
        } else {
            ValidationResult::Allowed
        }
    }

    /// Get the revoked components as a slice. The component version
    /// indicates the lowest *allowed* version of this component; all
    /// lower versions are considered revoked.
    pub fn revoked_components(&self) -> &[Component<'a>] {
        self.components.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Vendor;
    use crate::Generation;
    use arrayvec::ArrayVec;

    fn ascii(s: &str) -> &AsciiStr {
        AsciiStr::from_ascii(s).unwrap()
    }

    fn make_component(name: &str, gen: u32) -> Component {
        Component::new(ascii(name), Generation::new(gen).unwrap())
    }

    fn make_entry(name: &str, gen: u32) -> Entry {
        Entry::new(make_component(name, gen), Vendor::default())
    }

    fn make_metadata<'a>(
        components: &'a [Component<'a>],
    ) -> Metadata<'a, ArrayVec<Entry<'a>, 10>> {
        let mut entries = ArrayVec::<_, 10>::new();
        for comp in components {
            entries.push(Entry::new(comp.clone(), Vendor::default()));
        }

        Metadata::new(entries)
    }

    fn make_revocations<'a, 'b>(
        data: &'a [Component<'b>],
    ) -> Revocations<'b, ArrayVec<Component<'b>, 10>> {
        let mut revocations = ArrayVec::<_, 10>::new();
        for elem in data {
            revocations.push(elem.clone());
        }

        Revocations::new(revocations)
    }

    #[test]
    fn parse_success() {
        let input = b"sbat,1,2021030218\ncompA,1\ncompB,2";

        let array = ArrayVec::<_, 3>::new();
        let mut revocations = Revocations::new(array);
        revocations.parse(input).unwrap();

        assert_eq!(revocations.date, Some(ascii("2021030218")));

        assert_eq!(
            revocations.revoked_components(),
            [
                make_component("sbat", 1),
                make_component("compA", 1),
                make_component("compB", 2)
            ],
        );
    }

    #[test]
    fn too_few_fields() {
        let input = b"sbat";

        let array = ArrayVec::<_, 2>::new();
        let mut revocations = Revocations::new(array);
        assert_eq!(revocations.parse(input), Err(Error::TooFewFields));
    }

    #[test]
    fn no_date_field() {
        let input = b"sbat,1";

        let array = ArrayVec::<_, 1>::new();
        let mut revocations = Revocations::new(array);
        revocations.parse(input).unwrap();

        assert!(revocations.date.is_none());

        assert_eq!(
            revocations.revoked_components(),
            [make_component("sbat", 1)]
        );
    }

    #[test]
    fn is_component_revoked() {
        let revocations = make_revocations(&[
            make_component("compA", 2),
            make_component("compB", 3),
        ]);

        // compA: anything less than 2 is invalid.
        assert!(revocations.is_component_revoked(&make_component("compA", 1)));
        assert!(!revocations.is_component_revoked(&make_component("compA", 2)));
        assert!(!revocations.is_component_revoked(&make_component("compA", 3)));

        // compB: anything less than 3 is invalid.
        assert!(revocations.is_component_revoked(&make_component("compB", 2)));
        assert!(!revocations.is_component_revoked(&make_component("compB", 3)));
        assert!(!revocations.is_component_revoked(&make_component("compB", 4)));

        // compC: anything is valid.
        assert!(!revocations.is_component_revoked(&make_component("compC", 1)));
        assert!(!revocations.is_component_revoked(&make_component("compC", 2)));
        assert!(!revocations.is_component_revoked(&make_component("compC", 3)));
    }

    #[test]
    fn validate_metadata() {
        use ValidationResult::{Allowed, Revoked};

        let revocations = make_revocations(&[
            make_component("compA", 2),
            make_component("compB", 3),
        ]);

        // Invalid component.
        assert_eq!(
            revocations.validate_metadata(&make_metadata(&[make_component(
                "compA", 1
            )])),
            Revoked(&make_entry("compA", 1))
        );

        // compA valid, compB invalid.
        assert_eq!(
            revocations.validate_metadata(&make_metadata(&[
                make_component("compA", 2),
                make_component("compB", 2),
            ])),
            Revoked(&make_entry("compB", 2))
        );

        // compA invalid, compB valid.
        assert_eq!(
            revocations.validate_metadata(&make_metadata(&[
                make_component("compA", 1),
                make_component("compB", 3),
            ])),
            Revoked(&make_entry("compA", 1))
        );

        // compA valid, compB valid.
        assert_eq!(
            revocations.validate_metadata(&make_metadata(&[
                make_component("compA", 2),
                make_component("compB", 3),
            ])),
            Allowed
        );

        // compC valid.
        assert_eq!(
            revocations.validate_metadata(&make_metadata(&[make_component(
                "compC", 1
            )])),
            Allowed
        );

        // compC valid, compA invalid.
        assert_eq!(
            revocations.validate_metadata(&make_metadata(&[
                make_component("compC", 1),
                make_component("compA", 1)
            ])),
            Revoked(&make_entry("compA", 1))
        );
    }

    /// Test that `Revocations::new` does not clear the storage, and test
    /// that `Revocations::parse` does clear the storage.
    #[test]
    fn storage_clear() {
        let mut array = ArrayVec::<_, 2>::new();
        array.push(Component::default());

        // Initially the input storage has one component, which should stay
        // true after calling `new`.
        let mut revocations = Revocations::new(array);
        assert_eq!(revocations.revoked_components().len(), 1);

        // Calling parse should clear out the existing data.
        revocations.parse(b"").unwrap();
        assert!(revocations.revoked_components().is_empty());
    }
}
