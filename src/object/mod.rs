//! MTP object (can be a folder, a file, etc.)

use std::path::{Path, Components, Component};
use std::iter::Peekable;

use windows::core::{GUID, PWSTR, PCWSTR};
use windows::Win32::Devices::PortableDevices::WPD_OBJECT_PARENT_ID;
use widestring::{U16CString, U16CStr};

use crate::device::Content;
use crate::error::ItemByPathError;

mod object_id;
pub use object_id::ObjectId;

mod object_type;
pub use object_type::ObjectType;

mod object_iterator;
pub use object_iterator::ObjectIterator;


#[derive(Debug, Clone)]
pub struct Object {
    device_content: Content,
    /// The MTP ID of the object (e.g. "o2C")
    id: U16CString,
    /// The object display name (e.g. "PIC_001.jpg")
    name: U16CString,
    ty: ObjectType,
}

impl Object {
    pub fn new(device_content: Content, id: U16CString, name: U16CString, ty: ObjectType) -> Self {
        Self { device_content, id, name, ty }
    }

    pub(crate) fn device_content(&self) -> &Content {
        &self.device_content
    }

    pub fn id(&self) -> &U16CStr {
        &self.id
    }

    pub fn name(&self) -> &U16CStr {
        // TODO: lazy evaluation (of all properties at once to save calls to properties.GetValues) (depends on how much iterating/filtering by folder is baked-in)?
        &self.name
    }

    pub fn object_type(&self) -> ObjectType {
        // TODO: lazy evaluation?
        self.ty
    }

    pub fn parent_id(&self) -> crate::WindowsResult<U16CString> {
        let parent_id_props = self.device_content.get_object_properties(&self.id, &[WPD_OBJECT_PARENT_ID])?;
        let parent_id_pwstr = unsafe{ parent_id_props.GetStringValue(&WPD_OBJECT_PARENT_ID as *const _) }?;
        Ok(U16CString::from_vec_truncate(unsafe{ parent_id_pwstr.as_wide() }))
    }

    /// Returns an iterator to list every children of the current object (including sub-folders)
    pub fn children(&self) -> crate::WindowsResult<ObjectIterator> {
        let com_iter = unsafe{
            self.device_content.com_object().EnumObjects(
                0,
                PCWSTR::from_raw(self.id.as_ptr()),
                None,
            )
        }?;

        Ok(ObjectIterator::new(&self.device_content, com_iter))
    }

    /// Returns an iterator that only lists folders within this object
    pub fn sub_folders(&self) -> crate::WindowsResult<impl Iterator<Item = Object> + '_> {
        self.children().map(|children| children.filter(|obj| obj.object_type() == ObjectType::Folder))
    }

    /// Retrieve an item by its path
    ///
    /// This function looks for a sub-item with the right name, then iteratively does so for the matching child.<br/>
    /// This is quite expensive. Depending on your use-cases, you may want to cache some "parent folders" that you will want to access often.<br/>
    /// Note that caching however defeats the purpose of MTP, which is supposed to _not_ use any cache, so that it guarantees there is no race between concurrent accesses to the same medium.
    pub fn object_by_path(&self, relative_path: &Path) -> Result<Object, ItemByPathError> {
        let mut comps = relative_path.components().peekable();
        self.object_by_components(&mut comps)
    }

    fn object_by_components(&self, comps: &mut Peekable<Components>) -> Result<Object, ItemByPathError> {
        match comps.next() {
            Some(Component::Normal(name)) => {
                let haystack = U16CString::from_os_str_truncate(name);
                let candidate = self
                    .children()?
                    .find(|obj| obj.name() == haystack)
                    .ok_or(ItemByPathError::NotFound)?;

                object_by_components_last_stage(candidate, comps)
            },

            Some(Component::CurDir) => {
                object_by_components_last_stage(self.clone(), comps)
            },

            Some(Component::ParentDir) => {
                let candidate = self
                    .device_content
                    .object_by_id(self.parent_id()?)?;

                object_by_components_last_stage(candidate, comps)
            }

            Some(Component::Prefix(_)) |
            Some(Component::RootDir) =>
                Err(ItemByPathError::AbsolutePath),

            None => Err(ItemByPathError::NotFound)
        }
    }
}

fn object_by_components_last_stage(candidate: Object, next_components: &mut Peekable<Components>) -> Result<Object, ItemByPathError> {
    match next_components.peek() {
        None => {
            // We've reached the end of the required path
            // This means the candidate is the object we wanted
            Ok(candidate)
        },
        Some(_) => {
            candidate.object_by_components(next_components)
        }
    }
}
