use std::{
    collections::{HashMap, VecDeque},
    convert::TryInto,
    io::{self, Read},
    str,
};

use rbx_dom_weak::{
    types::{
        Axes, BinaryString, BrickColor, CFrame, Color3, Color3uint8, ColorSequence,
        ColorSequenceKeypoint, Content, CustomPhysicalProperties, EnumValue, Faces, Matrix3,
        NumberRange, NumberSequence, NumberSequenceKeypoint, PhysicalProperties, Ray, Rect, Ref,
        SharedString, UDim, UDim2, Variant, VariantType, Vector2, Vector3, Vector3int16,
    },
    InstanceBuilder, WeakDom,
};
use rbx_reflection::DataType;
use thiserror::Error;

use crate::{
    chunk::Chunk,
    core::{
        find_canonical_property_descriptor, RbxReadExt, FILE_MAGIC_HEADER, FILE_SIGNATURE,
        FILE_VERSION,
    },
    types::{InvalidTypeError, Type},
};

/// Represents an error that occurred during deserialization.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct Error {
    source: Box<InnerError>,
}

impl From<InnerError> for Error {
    fn from(inner: InnerError) -> Self {
        Self {
            source: Box::new(inner),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum InnerError {
    #[error(transparent)]
    Io {
        #[from]
        source: io::Error,
    },

    #[error("Invalid file header")]
    BadHeader,

    #[error("Unknown file version {version}. Known versions are: 0")]
    UnknownFileVersion { version: u16 },

    #[error("Unknown version {version} for chunk {chunk_name}")]
    UnknownChunkVersion {
        chunk_name: &'static str,
        version: u32,
    },

    #[error(transparent)]
    InvalidTypeError {
        #[from]
        source: InvalidTypeError,
    },

    #[error(
        "Type mismatch: Property {type_name}.{prop_name} should be {valid_type_names}, but it was {actual_type_name}",
    )]
    PropTypeMismatch {
        type_name: String,
        prop_name: String,
        valid_type_names: &'static str,
        actual_type_name: String,
    },

    #[error("Invalid property data: Property {type_name}.{prop_name} was expected to be {valid_value}, but it was {actual_value}")]
    InvalidPropData {
        type_name: String,
        prop_name: String,
        valid_value: &'static str,
        actual_value: String,
    },

    #[error("File referred to type ID {type_id}, which was not declared")]
    InvalidTypeId { type_id: u32 },

    #[error("Invalid property data: CFrame property {type_name}.{prop_name} had an invalid orientation ID {id:02x}")]
    BadCFrameOrientationId {
        type_name: String,
        prop_name: String,
        id: u8,
    },
}

// TODO potentially move this to a different file if/when we do the inverse for serializing
pub(crate) fn special_case_to_rotation(id: u8) -> Option<Matrix3> {
    match id {
        0x02 => Some(Matrix3::new(
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
        )),
        0x03 => Some(Matrix3::new(
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(0.0, 1.0, 0.0),
        )),
        0x05 => Some(Matrix3::new(
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
        )),
        0x06 => Some(Matrix3::new(
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.0, -1.0, 0.0),
        )),
        0x07 => Some(Matrix3::new(
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
        )),
        0x09 => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
        )),
        0x0a => Some(Matrix3::new(
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
        )),
        0x0c => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, -1.0, 0.0),
        )),
        0x0d => Some(Matrix3::new(
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(1.0, 0.0, 0.0),
        )),
        0x0e => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
        )),
        0x10 => Some(Matrix3::new(
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(1.0, 0.0, 0.0),
        )),
        0x11 => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
        )),
        0x14 => Some(Matrix3::new(
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
        )),
        0x15 => Some(Matrix3::new(
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.0, 1.0, 0.0),
        )),
        0x17 => Some(Matrix3::new(
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
        )),
        0x18 => Some(Matrix3::new(
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(0.0, -1.0, 0.0),
        )),
        0x19 => Some(Matrix3::new(
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
        )),
        0x1b => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
        )),
        0x1c => Some(Matrix3::new(
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
        )),
        0x1e => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, -1.0, 0.0),
        )),
        0x1f => Some(Matrix3::new(
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(-1.0, 0.0, 0.0),
        )),
        0x20 => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
        )),
        0x22 => Some(Matrix3::new(
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(-1.0, 0.0, 0.0),
        )),
        0x23 => Some(Matrix3::new(
            Vector3::new(0.0, 0.0, -1.0),
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
        )),
        _ => None,
    }
}

pub(crate) fn decode<R: Read>(reader: R) -> Result<WeakDom, Error> {
    Ok(decode_inner(reader)?)
}

pub(crate) fn decode_inner<R: Read>(reader: R) -> Result<WeakDom, InnerError> {
    let mut deserializer = BinaryDeserializer::new(reader)?;

    loop {
        let chunk = Chunk::decode(&mut deserializer.input)?;

        match &chunk.name {
            b"META" => deserializer.decode_meta_chunk(&chunk.data)?,
            b"SSTR" => deserializer.decode_sstr_chunk(&chunk.data)?,
            b"INST" => deserializer.decode_inst_chunk(&chunk.data)?,
            b"PROP" => deserializer.decode_prop_chunk(&chunk.data)?,
            b"PRNT" => deserializer.decode_prnt_chunk(&chunk.data)?,
            b"END\0" => {
                deserializer.decode_end_chunk(&chunk.data)?;
                break;
            }
            _ => match str::from_utf8(&chunk.name) {
                Ok(name) => log::info!("Unknown binary chunk name {}", name),
                Err(_) => log::info!("Unknown binary chunk name {:?}", chunk.name),
            },
        }
    }

    Ok(deserializer.finish())
}

struct BinaryDeserializer<R> {
    /// The input data encoded as a binary model.
    input: R,

    /// The tree that instances should be written into. Eventually returned to
    /// the user.
    tree: WeakDom,

    /// The metadata contained in the file, which affects how some constructs
    /// are interpreted by Roblox.
    metadata: HashMap<String, String>,

    /// The SharedStrings contained in the file, if any, in the order that they
    /// appear in the file.
    shared_strings: Vec<SharedString>,

    /// All of the instance types described by the file so far.
    type_infos: HashMap<u32, TypeInfo>,

    /// All of the instances known by the deserializer.
    instances_by_ref: HashMap<i32, Instance>,

    /// Referents for all of the instances with no parent, in order they appear
    /// in the file.
    root_instance_refs: Vec<i32>,
}

/// All the information contained in the header before any chunks are read from
/// the file.
pub(crate) struct FileHeader {
    /// The number of instance types (represented for us as `TypeInfo`) that are
    /// in this file. Generally useful to pre-size some containers before
    /// reading the file.
    pub(crate) num_types: u32,

    /// The total number of instances described by this file.
    pub(crate) num_instances: u32,
}

/// Represents a unique instance class. Binary models define all their instance
/// types up front and give them a short u32 identifier.
struct TypeInfo {
    /// The ID given to this type by the current file we're deserializing. This
    /// ID can be different for different files.
    type_id: u32,

    /// The common name for this type like `Folder` or `UserInputService`.
    type_name: String,

    /// A list of the instances described by this file that are this type.
    referents: Vec<i32>,
}

/// Contains all the information we need to gather in order to construct an
/// instance. Incrementally built up by the deserializer as we decode different
/// chunks.
struct Instance {
    /// A work-in-progress builder that will be used to construct this instance.
    builder: InstanceBuilder,

    /// Document-defined IDs for the children of this instance.
    children: Vec<i32>,
}

impl<R: Read> BinaryDeserializer<R> {
    fn new(mut input: R) -> Result<Self, InnerError> {
        let tree = WeakDom::new(InstanceBuilder::new("DataModel"));

        let header = FileHeader::decode(&mut input)?;

        let type_infos = HashMap::with_capacity(header.num_types as usize);
        let instances_by_ref = HashMap::with_capacity(1 + header.num_instances as usize);

        Ok(BinaryDeserializer {
            input,
            tree,
            metadata: HashMap::new(),
            shared_strings: Vec::new(),
            type_infos,
            instances_by_ref,
            root_instance_refs: Vec::new(),
        })
    }

    fn decode_meta_chunk(&mut self, mut chunk: &[u8]) -> Result<(), InnerError> {
        let len = chunk.read_le_u32()?;
        self.metadata.reserve(len as usize);

        for _ in 0..len {
            let key = chunk.read_string()?;
            let value = chunk.read_string()?;

            self.metadata.insert(key, value);
        }

        Ok(())
    }

    fn decode_sstr_chunk(&mut self, mut chunk: &[u8]) -> Result<(), InnerError> {
        let version = chunk.read_le_u32()?;

        if version != 0 {
            return Err(InnerError::UnknownChunkVersion {
                chunk_name: "SSTR",
                version,
            });
        }

        let num_entries = chunk.read_le_u32()?;

        for _ in 0..num_entries {
            chunk.read_exact(&mut [0; 16])?; // We don't do anything with the hash.
            let data = chunk.read_binary_string()?;
            self.shared_strings.push(SharedString::new(data));
        }

        Ok(())
    }

    fn decode_inst_chunk(&mut self, mut chunk: &[u8]) -> Result<(), InnerError> {
        let type_id = chunk.read_le_u32()?;
        let type_name = chunk.read_string()?;
        let object_format = chunk.read_u8()?;
        let number_instances = chunk.read_le_u32()?;

        log::trace!(
            "INST chunk (type ID {}, type name {}, format {}, {} instances)",
            type_id,
            type_name,
            object_format,
            number_instances,
        );

        let mut referents = vec![0; number_instances as usize];
        chunk.read_referent_array(&mut referents)?;

        // TODO: Check object_format and check for service markers if it's 1?

        for &referent in &referents {
            self.instances_by_ref.insert(
                referent,
                Instance {
                    builder: InstanceBuilder::new(&type_name),
                    children: Vec::new(),
                },
            );
        }

        self.type_infos.insert(
            type_id,
            TypeInfo {
                type_id,
                type_name,
                referents,
            },
        );

        Ok(())
    }

    fn decode_prop_chunk(&mut self, mut chunk: &[u8]) -> Result<(), InnerError> {
        let type_id = chunk.read_le_u32()?;
        let prop_name = chunk.read_string()?;
        let binary_type: Type = chunk.read_u8()?.try_into()?;

        let type_info = self
            .type_infos
            .get(&type_id)
            .ok_or(InnerError::InvalidTypeId { type_id })?;

        log::trace!(
            "PROP chunk ({}.{}, instance type {}, prop type {}",
            type_info.type_name,
            prop_name,
            type_info.type_id,
            type_id
        );

        // The `Name` prop is special and is routed to a different spot for
        // rbx_dom_weak, so we handle it specially here.
        if prop_name == "Name" {
            // TODO: If an instance is never assigned a name through this code
            // path, we should use the reflection database to figure out its
            // default name. This should be rare: effectively never!

            for referent in &type_info.referents {
                let instance = self.instances_by_ref.get_mut(referent).unwrap();
                let value = chunk.read_string()?;
                instance.builder.set_name(value);
            }

            return Ok(());
        }

        let canonical_name;
        let canonical_type;

        match find_canonical_property_descriptor(&type_info.type_name, &prop_name) {
            Some(descriptor) => {
                canonical_name = descriptor.name.clone().into_owned();
                canonical_type = match &descriptor.data_type {
                    DataType::Value(ty) => *ty,
                    DataType::Enum(_) => VariantType::EnumValue,
                    _ => {
                        // TODO: Configurable handling of unknown types?
                        return Ok(());
                    }
                };

                log::trace!(
                    "Known prop, canonical name {} and type {:?}",
                    canonical_name,
                    canonical_type
                );
            }
            None => {
                canonical_name = prop_name.clone();

                match binary_type.to_default_rbx_type() {
                    Some(rbx_type) => canonical_type = rbx_type,
                    None => {
                        log::warn!("Unsupported prop type {:?}, skipping property", binary_type);

                        return Ok(());
                    }
                }

                log::trace!("Unknown prop, using type {:?}", canonical_type);
            }
        }

        match binary_type {
            Type::String => match canonical_type {
                VariantType::String => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value = chunk.read_string()?;
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                VariantType::Content => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value: Content = chunk.read_string()?.into();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                VariantType::BinaryString => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value: BinaryString = chunk.read_binary_string()?.into();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "String, Content, or BinaryString",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Bool => match canonical_type {
                VariantType::Bool => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value = chunk.read_bool()?;
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Bool",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Int32 => match canonical_type {
                VariantType::Int32 => {
                    let mut values = vec![0; type_info.referents.len()];
                    chunk.read_interleaved_i32_array(&mut values)?;

                    for (value, referent) in values.into_iter().zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Int32",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Float32 => match canonical_type {
                VariantType::Float32 => {
                    let mut values = vec![0.0; type_info.referents.len()];
                    chunk.read_interleaved_f32_array(&mut values)?;

                    for (value, referent) in values.into_iter().zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Float32",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Float64 => match canonical_type {
                VariantType::Float64 => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value = chunk.read_le_f64()?;
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Float64",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::UDim => match canonical_type {
                VariantType::UDim => {
                    let mut scales = vec![0.0; type_info.referents.len()];
                    let mut offsets = vec![0; type_info.referents.len()];

                    chunk.read_interleaved_f32_array(&mut scales)?;
                    chunk.read_interleaved_i32_array(&mut offsets)?;

                    let values = scales
                        .into_iter()
                        .zip(offsets)
                        .map(|(scale, offset)| UDim::new(scale, offset));

                    for (value, referent) in values.zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "UDim",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::UDim2 => match canonical_type {
                VariantType::UDim2 => {
                    let prop_count = type_info.referents.len();
                    let mut scale_x = vec![0.0; prop_count];
                    let mut scale_y = vec![0.0; prop_count];
                    let mut offset_x = vec![0; prop_count];
                    let mut offset_y = vec![0; prop_count];

                    chunk.read_interleaved_f32_array(&mut scale_x)?;
                    chunk.read_interleaved_f32_array(&mut scale_y)?;
                    chunk.read_interleaved_i32_array(&mut offset_x)?;
                    chunk.read_interleaved_i32_array(&mut offset_y)?;

                    let x = scale_x
                        .into_iter()
                        .zip(offset_x)
                        .map(|(scale, offset)| UDim::new(scale, offset));

                    let y = scale_y
                        .into_iter()
                        .zip(offset_y)
                        .map(|(scale, offset)| UDim::new(scale, offset));

                    let values = x.zip(y).map(|(x, y)| UDim2::new(x, y));

                    for (value, referent) in values.zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "UDim2",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Ray => match canonical_type {
                VariantType::Ray => {
                    for referent in &type_info.referents {
                        let origin_x = chunk.read_le_f32()?;
                        let origin_y = chunk.read_le_f32()?;
                        let origin_z = chunk.read_le_f32()?;
                        let direction_x = chunk.read_le_f32()?;
                        let direction_y = chunk.read_le_f32()?;
                        let direction_z = chunk.read_le_f32()?;

                        let instance = self.instances_by_ref.get_mut(referent).unwrap();

                        instance.builder.add_property(
                            &canonical_name,
                            Ray::new(
                                Vector3::new(origin_x, origin_y, origin_z),
                                Vector3::new(direction_x, direction_y, direction_z),
                            ),
                        );
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Ray",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Faces => match canonical_type {
                VariantType::Faces => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value = chunk.read_u8()?;
                        let faces =
                            Faces::from_bits(value).ok_or_else(|| InnerError::InvalidPropData {
                                type_name: type_info.type_name.clone(),
                                prop_name: prop_name.clone(),
                                valid_value: "less than 63",
                                actual_value: value.to_string(),
                            })?;

                        instance.builder.add_property(&canonical_name, faces);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Faces",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Axes => match canonical_type {
                VariantType::Axes => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value = chunk.read_u8()?;

                        let axes =
                            Axes::from_bits(value).ok_or_else(|| InnerError::InvalidPropData {
                                type_name: type_info.type_name.clone(),
                                prop_name: prop_name.clone(),
                                valid_value: "less than 7",
                                actual_value: value.to_string(),
                            })?;

                        instance.builder.add_property(&canonical_name, axes);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Axes",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::BrickColor => match canonical_type {
                VariantType::BrickColor => {
                    let mut values = vec![0; type_info.referents.len()];
                    chunk.read_interleaved_u32_array(&mut values)?;

                    for (value, referent) in values.into_iter().zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let color = value
                            .try_into()
                            .ok()
                            .and_then(BrickColor::from_number)
                            .ok_or_else(|| InnerError::InvalidPropData {
                                type_name: type_info.type_name.clone(),
                                prop_name: prop_name.clone(),
                                valid_value: "a valid BrickColor",
                                actual_value: value.to_string(),
                            })?;

                        instance.builder.add_property(&canonical_name, color);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "BrickColor",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Color3 => match canonical_type {
                VariantType::Color3 => {
                    let mut r = vec![0.0; type_info.referents.len()];
                    let mut g = vec![0.0; type_info.referents.len()];
                    let mut b = vec![0.0; type_info.referents.len()];

                    chunk.read_interleaved_f32_array(&mut r)?;
                    chunk.read_interleaved_f32_array(&mut g)?;
                    chunk.read_interleaved_f32_array(&mut b)?;

                    let colors = r
                        .into_iter()
                        .zip(g)
                        .zip(b)
                        .map(|((r, g), b)| Color3::new(r, g, b));

                    for (color, referent) in colors.zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, color);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Color3",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Vector2 => match canonical_type {
                VariantType::Vector2 => {
                    let mut x = vec![0.0; type_info.referents.len()];
                    let mut y = vec![0.0; type_info.referents.len()];

                    chunk.read_interleaved_f32_array(&mut x)?;
                    chunk.read_interleaved_f32_array(&mut y)?;

                    let values = x.into_iter().zip(y).map(|(x, y)| Vector2::new(x, y));

                    for (value, referent) in values.zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Vector2",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Vector3 => match canonical_type {
                VariantType::Vector3 => {
                    let mut x = vec![0.0; type_info.referents.len()];
                    let mut y = vec![0.0; type_info.referents.len()];
                    let mut z = vec![0.0; type_info.referents.len()];

                    chunk.read_interleaved_f32_array(&mut x)?;
                    chunk.read_interleaved_f32_array(&mut y)?;
                    chunk.read_interleaved_f32_array(&mut z)?;

                    let values = x
                        .into_iter()
                        .zip(y)
                        .zip(z)
                        .map(|((x, y), z)| Vector3::new(x, y, z));

                    for (value, referent) in values.zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Vector3",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::CFrame => match canonical_type {
                VariantType::CFrame => {
                    let referents = &type_info.referents;
                    let mut rotations = Vec::with_capacity(referents.len());

                    for _ in 0..referents.len() {
                        let id = chunk.read_u8()?;
                        if id == 0 {
                            rotations.push(Matrix3::new(
                                Vector3::new(
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                ),
                                Vector3::new(
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                ),
                                Vector3::new(
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                ),
                            ));
                        } else {
                            let special_case = special_case_to_rotation(id).ok_or_else(|| {
                                InnerError::BadCFrameOrientationId {
                                    type_name: type_info.type_name.clone(),
                                    prop_name: prop_name.clone(),
                                    id,
                                }
                            })?;

                            rotations.push(special_case);
                        }
                    }

                    let mut x = vec![0.0; referents.len()];
                    let mut y = vec![0.0; referents.len()];
                    let mut z = vec![0.0; referents.len()];

                    chunk.read_interleaved_f32_array(&mut x)?;
                    chunk.read_interleaved_f32_array(&mut y)?;
                    chunk.read_interleaved_f32_array(&mut z)?;

                    let values = x
                        .into_iter()
                        .zip(y)
                        .zip(z)
                        .map(|((x, y), z)| Vector3::new(x, y, z))
                        .zip(rotations)
                        .map(|(position, rotation)| CFrame::new(position, rotation));

                    for (cframe, referent) in values.zip(referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, cframe);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "CFrame",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Enum => match canonical_type {
                VariantType::EnumValue => {
                    let mut values = vec![0; type_info.referents.len()];
                    chunk.read_interleaved_u32_array(&mut values)?;

                    for (value, referent) in values.into_iter().zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance
                            .builder
                            .add_property(&canonical_name, EnumValue::from_u32(value));
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Enum",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Ref => match canonical_type {
                VariantType::Ref => {
                    let mut refs = vec![0; type_info.referents.len()];
                    chunk.read_referent_array(&mut refs)?;

                    for (value, referent) in refs.into_iter().zip(&type_info.referents) {
                        let rbx_value = if let Some(instance) = self.instances_by_ref.get(&value) {
                            instance.builder.referent()
                        } else {
                            Ref::none()
                        };

                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, rbx_value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Ref",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Vector3int16 => match canonical_type {
                VariantType::Vector3int16 => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(
                            &canonical_name,
                            Vector3int16::new(
                                chunk.read_le_i16()?,
                                chunk.read_le_i16()?,
                                chunk.read_le_i16()?,
                            ),
                        )
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Vector3int16",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::NumberSequence => match canonical_type {
                VariantType::NumberSequence => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let keypoint_count = chunk.read_le_u32()?;
                        let mut keypoints = Vec::with_capacity(keypoint_count as usize);

                        for _ in 0..keypoint_count {
                            keypoints.push(NumberSequenceKeypoint::new(
                                chunk.read_le_f32()?,
                                chunk.read_le_f32()?,
                                chunk.read_le_f32()?,
                            ))
                        }

                        instance
                            .builder
                            .add_property(&canonical_name, NumberSequence { keypoints })
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "NumberSequence",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::ColorSequence => match canonical_type {
                VariantType::ColorSequence => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let keypoint_count = chunk.read_le_u32()? as usize;
                        let mut keypoints = Vec::with_capacity(keypoint_count);

                        for _ in 0..keypoint_count {
                            keypoints.push(ColorSequenceKeypoint::new(
                                chunk.read_le_f32()?,
                                Color3::new(
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                    chunk.read_le_f32()?,
                                ),
                            ));

                            // envelope is serialized but doesn't do anything; don't do anything with it
                            chunk.read_le_f32()?;
                        }

                        instance
                            .builder
                            .add_property(&canonical_name, ColorSequence { keypoints })
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "ColorSequence",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::NumberRange => match canonical_type {
                VariantType::NumberRange => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(
                            &canonical_name,
                            NumberRange::new(chunk.read_le_f32()?, chunk.read_le_f32()?),
                        )
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "NumberRange",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Rect => match canonical_type {
                VariantType::Rect => {
                    let len = type_info.referents.len();
                    let mut x_min = vec![0.0; len];
                    let mut y_min = vec![0.0; len];
                    let mut x_max = vec![0.0; len];
                    let mut y_max = vec![0.0; len];

                    chunk.read_interleaved_f32_array(&mut x_min)?;
                    chunk.read_interleaved_f32_array(&mut y_min)?;
                    chunk.read_interleaved_f32_array(&mut x_max)?;
                    chunk.read_interleaved_f32_array(&mut y_max)?;

                    let values = x_min.into_iter().zip(y_min).zip(x_max).zip(y_max).map(
                        |(((x_min, y_min), x_max), y_max)| {
                            Rect::new(Vector2::new(x_min, y_min), Vector2::new(x_max, y_max))
                        },
                    );

                    for (value, referent) in values.zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value)
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Rect",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::PhysicalProperties => match canonical_type {
                VariantType::PhysicalProperties => {
                    for referent in &type_info.referents {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        let value = if chunk.read_u8()? == 1 {
                            Variant::PhysicalProperties(PhysicalProperties::Custom(
                                CustomPhysicalProperties {
                                    density: chunk.read_le_f32()?,
                                    friction: chunk.read_le_f32()?,
                                    elasticity: chunk.read_le_f32()?,
                                    friction_weight: chunk.read_le_f32()?,
                                    elasticity_weight: chunk.read_le_f32()?,
                                },
                            ))
                        } else {
                            Variant::PhysicalProperties(PhysicalProperties::Default)
                        };

                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "PhysicalProperties",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Color3uint8 => match canonical_type {
                VariantType::Color3 => {
                    let len = type_info.referents.len();
                    let mut r = vec![0; len];
                    let mut g = vec![0; len];
                    let mut b = vec![0; len];

                    chunk.read_exact(r.as_mut_slice())?;
                    chunk.read_exact(g.as_mut_slice())?;
                    chunk.read_exact(b.as_mut_slice())?;

                    let colors = r
                        .into_iter()
                        .zip(g)
                        .zip(b)
                        .map(|((r, g), b)| Color3uint8::new(r, g, b));

                    for (color, referent) in colors.into_iter().zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, color);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Color3",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::Int64 => match canonical_type {
                VariantType::Int64 => {
                    let mut values = vec![0; type_info.referents.len()];
                    chunk.read_interleaved_i64_array(&mut values)?;

                    for (value, referent) in values.into_iter().zip(&type_info.referents) {
                        let instance = self.instances_by_ref.get_mut(referent).unwrap();
                        instance.builder.add_property(&canonical_name, value);
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "Int64",
                        actual_type_name: format!("{:?}", invalid_type),
                    });
                }
            },
            Type::SharedString => match canonical_type {
                VariantType::SharedString => {
                    let mut values = vec![0; type_info.referents.len()];
                    chunk.read_interleaved_u32_array(&mut values)?;

                    for (value, referent) in values.into_iter().zip(&type_info.referents) {
                        let shared_string =
                            self.shared_strings.get(value as usize).ok_or_else(|| {
                                InnerError::InvalidPropData {
                                    type_name: type_info.type_name.clone(),
                                    prop_name: prop_name.clone(),
                                    valid_value: "a valid SharedString",
                                    actual_value: format!("{:?}", value),
                                }
                            })?;

                        let instance = self.instances_by_ref.get_mut(referent).unwrap();

                        instance
                            .builder
                            .add_property(&canonical_name, shared_string.clone());
                    }
                }
                invalid_type => {
                    return Err(InnerError::PropTypeMismatch {
                        type_name: type_info.type_name.clone(),
                        prop_name,
                        valid_type_names: "SharedString",
                        actual_type_name: format!("{:?}", invalid_type),
                    })
                }
            },
        }

        Ok(())
    }

    fn decode_prnt_chunk(&mut self, mut chunk: &[u8]) -> Result<(), InnerError> {
        let version = chunk.read_u8()?;

        if version != 0 {
            return Err(InnerError::UnknownChunkVersion {
                chunk_name: "PRNT",
                version: version as u32,
            });
        }

        let number_objects = chunk.read_le_u32()?;

        log::trace!("PRNT chunk ({} instances)", number_objects);

        let mut subjects = vec![0; number_objects as usize];
        let mut parents = vec![0; number_objects as usize];

        chunk.read_referent_array(&mut subjects)?;
        chunk.read_referent_array(&mut parents)?;

        for (id, parent_ref) in subjects.iter().copied().zip(parents.iter().copied()) {
            if parent_ref == -1 {
                self.root_instance_refs.push(id);
            } else {
                let instance = self.instances_by_ref.get_mut(&parent_ref).unwrap();
                instance.children.push(id);
            }
        }

        Ok(())
    }

    fn decode_end_chunk(&mut self, _chunk: &[u8]) -> Result<(), InnerError> {
        log::trace!("END chunk");

        // We don't do any validation on the END chunk. There's no useful
        // information for us here as it just signals that the file hasn't been
        // truncated.

        Ok(())
    }

    /// Combines together all the decoded information to build and emplace
    /// instances in our tree.
    fn finish(mut self) -> WeakDom {
        log::trace!("Constructing tree from deserialized data");

        // Track all the instances we need to construct. Order of construction
        // is important to preserve for both determinism and sometimes
        // functionality of models we handle.
        let mut instances_to_construct = VecDeque::new();

        // Any instance with a parent of -1 will be at the top level of the
        // tree. Because of the way rbx_dom_weak generally works, we need to
        // start at the top of the tree to begin construction.
        let root_ref = self.tree.root_ref();
        for &referent in &self.root_instance_refs {
            instances_to_construct.push_back((referent, root_ref));
        }

        while let Some((referent, parent_ref)) = instances_to_construct.pop_front() {
            let instance = self.instances_by_ref.remove(&referent).unwrap();
            let id = self.tree.insert(parent_ref, instance.builder);

            for referent in instance.children {
                instances_to_construct.push_back((referent, id));
            }
        }

        self.tree
    }
}

impl FileHeader {
    pub(crate) fn decode<R: Read>(mut source: R) -> Result<Self, InnerError> {
        let mut magic_header = [0; 8];
        source.read_exact(&mut magic_header)?;

        if magic_header != FILE_MAGIC_HEADER {
            return Err(InnerError::BadHeader);
        }

        let mut signature = [0; 6];
        source.read_exact(&mut signature)?;

        if signature != FILE_SIGNATURE {
            return Err(InnerError::BadHeader);
        }

        let version = source.read_le_u16()?;

        if version != FILE_VERSION {
            return Err(InnerError::UnknownFileVersion { version });
        }

        let num_types = source.read_le_u32()?;
        let num_instances = source.read_le_u32()?;

        let mut reserved = [0; 8];
        source.read_exact(&mut reserved)?;

        if reserved != [0; 8] {
            return Err(InnerError::BadHeader);
        }

        Ok(Self {
            num_types,
            num_instances,
        })
    }
}
