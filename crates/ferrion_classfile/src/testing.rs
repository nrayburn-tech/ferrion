//! Writers for small class files, for tests and fuzz seeds. Not part of the public API.
//!
//! The formats written here are the ones the readers in this crate accept, as documented in
//! docs/design/classfile-containers.md and docs/design/classfile-jimage.md. Nothing here is
//! copied from a JDK.

use std::collections::HashMap;

use crate::text;

/// Builds a class file: a constant pool, members, and attributes given as bytes.
#[derive(Default)]
pub struct ClassBuilder {
    pool: Vec<u8>,
    slots: u16,
    known: HashMap<Vec<u8>, u16>,
    pub major: u16,
    pub minor: u16,
    pub access: u16,
    this: u16,
    superclass: u16,
    interfaces: Vec<u16>,
    fields: Vec<u8>,
    field_count: u16,
    methods: Vec<u8>,
    method_count: u16,
    attributes: Vec<u8>,
    attribute_count: u16,
}

impl ClassBuilder {
    /// A class named `name` extending `java/lang/Object`, version 71.0, public.
    pub fn new(name: &str) -> ClassBuilder {
        let mut builder = ClassBuilder {
            slots: 1,
            major: 71,
            access: 0x0021,
            ..Default::default()
        };
        builder.this = builder.class(name);
        builder.superclass = builder.class("java/lang/Object");
        builder
    }

    /// Adds a constant pool entry (tag and contents), sharing equal entries.
    pub fn entry(&mut self, bytes: &[u8]) -> u16 {
        if let Some(&index) = self.known.get(bytes) {
            return index;
        }
        let index = self.slots;
        self.pool.extend_from_slice(bytes);
        self.slots += if matches!(bytes[0], 5 | 6) { 2 } else { 1 };
        self.known.insert(bytes.to_vec(), index);
        index
    }

    pub fn utf8_bytes(&mut self, bytes: &[u8]) -> u16 {
        let mut entry = vec![1];
        entry.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        entry.extend_from_slice(bytes);
        self.entry(&entry)
    }

    pub fn utf8(&mut self, text: &str) -> u16 {
        self.utf8_bytes(&text::encode(text))
    }

    fn indirect(&mut self, tag: u8, text: &str) -> u16 {
        let name = self.utf8(text);
        let [a, b] = name.to_be_bytes();
        self.entry(&[tag, a, b])
    }

    pub fn class(&mut self, name: &str) -> u16 {
        self.indirect(7, name)
    }

    pub fn string(&mut self, text: &str) -> u16 {
        self.indirect(8, text)
    }

    pub fn module(&mut self, name: &str) -> u16 {
        self.indirect(19, name)
    }

    pub fn package(&mut self, name: &str) -> u16 {
        self.indirect(20, name)
    }

    pub fn integer(&mut self, value: i32) -> u16 {
        let [a, b, c, d] = value.to_be_bytes();
        self.entry(&[3, a, b, c, d])
    }

    pub fn float(&mut self, bits: u32) -> u16 {
        let [a, b, c, d] = bits.to_be_bytes();
        self.entry(&[4, a, b, c, d])
    }

    pub fn long(&mut self, value: i64) -> u16 {
        let mut entry = vec![5];
        entry.extend_from_slice(&value.to_be_bytes());
        self.entry(&entry)
    }

    pub fn double(&mut self, bits: u64) -> u16 {
        let mut entry = vec![6];
        entry.extend_from_slice(&bits.to_be_bytes());
        self.entry(&entry)
    }

    pub fn name_and_type(&mut self, name: &str, descriptor: &str) -> u16 {
        let name = self.utf8(name);
        let descriptor = self.utf8(descriptor);
        let mut entry = vec![12];
        entry.extend_from_slice(&name.to_be_bytes());
        entry.extend_from_slice(&descriptor.to_be_bytes());
        self.entry(&entry)
    }

    pub fn set_super(&mut self, name: Option<&str>) {
        self.superclass = name.map_or(0, |name| self.class(name));
    }

    pub fn interface(&mut self, name: &str) {
        let index = self.class(name);
        self.interfaces.push(index);
    }

    /// An attribute: its name's index, length, and `contents`.
    pub fn attribute(&mut self, name: &str, contents: &[u8]) -> Vec<u8> {
        let mut bytes = self.utf8(name).to_be_bytes().to_vec();
        bytes.extend_from_slice(&(contents.len() as u32).to_be_bytes());
        bytes.extend_from_slice(contents);
        bytes
    }

    fn member(
        &mut self,
        access: u16,
        name: &str,
        descriptor: &str,
        attributes: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut bytes = access.to_be_bytes().to_vec();
        bytes.extend_from_slice(&self.utf8(name).to_be_bytes());
        bytes.extend_from_slice(&self.utf8(descriptor).to_be_bytes());
        bytes.extend_from_slice(&(attributes.len() as u16).to_be_bytes());
        for attribute in attributes {
            bytes.extend_from_slice(attribute);
        }
        bytes
    }

    pub fn field(&mut self, access: u16, name: &str, descriptor: &str, attributes: &[Vec<u8>]) {
        let member = self.member(access, name, descriptor, attributes);
        self.fields.extend_from_slice(&member);
        self.field_count += 1;
    }

    pub fn method(&mut self, access: u16, name: &str, descriptor: &str, attributes: &[Vec<u8>]) {
        let member = self.member(access, name, descriptor, attributes);
        self.methods.extend_from_slice(&member);
        self.method_count += 1;
    }

    pub fn class_attribute(&mut self, attribute: Vec<u8>) {
        self.attributes.extend_from_slice(&attribute);
        self.attribute_count += 1;
    }

    pub fn finish(&self) -> Vec<u8> {
        let mut bytes = 0xcafe_babe_u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(&self.minor.to_be_bytes());
        bytes.extend_from_slice(&self.major.to_be_bytes());
        bytes.extend_from_slice(&self.slots.to_be_bytes());
        bytes.extend_from_slice(&self.pool);
        bytes.extend_from_slice(&self.access.to_be_bytes());
        bytes.extend_from_slice(&self.this.to_be_bytes());
        bytes.extend_from_slice(&self.superclass.to_be_bytes());
        bytes.extend_from_slice(&(self.interfaces.len() as u16).to_be_bytes());
        for interface in &self.interfaces {
            bytes.extend_from_slice(&interface.to_be_bytes());
        }
        bytes.extend_from_slice(&self.field_count.to_be_bytes());
        bytes.extend_from_slice(&self.fields);
        bytes.extend_from_slice(&self.method_count.to_be_bytes());
        bytes.extend_from_slice(&self.methods);
        bytes.extend_from_slice(&self.attribute_count.to_be_bytes());
        bytes.extend_from_slice(&self.attributes);
        bytes
    }
}
