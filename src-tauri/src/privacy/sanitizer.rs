use dicom_core::header::Header;
use dicom_core::Tag;
use dicom_dictionary_std::tags;
use dicom_object::mem::InMemDicomObject;
use dicom_object::DefaultDicomObject;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PrivacyError {
    #[error("Zero-Egress Invariant Violation: PHI detected in payload: {0}")]
    PhiDetected(String),

    #[error("Forbidden asset access: Whole-slide label, macro, or overview camera requested")]
    MacroOverviewForbidden,

    #[error("Visual patch exceeds maximum dimension boundary of 1024x1024 ({0}x{1})")]
    PatchExceedsDimensionLimit(u32, u32),

    #[error("Invalid visual patch data: {0}")]
    InvalidPatch(String),

    #[error("DICOM processing error: {0}")]
    Dicom(String),
}

/// In-memory HIPAA Privacy & PHI Quarantine Sanitizer.
/// Enforces the Zero-Egress Invariant: No patient demographic data (0010,xxxx)
/// or institutional identifiers may ever cross network boundaries.
pub struct DicomSanitizer;

impl DicomSanitizer {
    /// Checks if a DICOM tag belongs to patient demographic or institutional PHI categories.
    pub fn is_phi_tag(tag: Tag) -> bool {
        let (group, _element) = (tag.0, tag.1);

        // Group 0x0010: Patient Identification and Demographics (ALL tags in this group are PHI)
        if group == 0x0010 {
            return true;
        }

        // Group 0x0008: Institutional, Study & Healthcare Provider Identifiers
        if group == 0x0008 {
            matches!(
                tag,
                tags::ACCESSION_NUMBER // (0008,0050)
                | tags::INSTITUTION_NAME // (0008,0080)
                | tags::INSTITUTION_ADDRESS // (0008,0081)
                | tags::REFERRING_PHYSICIAN_NAME // (0008,0090)
                | tags::REFERRING_PHYSICIAN_ADDRESS // (0008,0092)
                | tags::REFERRING_PHYSICIAN_TELEPHONE_NUMBERS // (0008,0094)
                | tags::INSTITUTIONAL_DEPARTMENT_NAME // (0008,1040)
                | tags::PHYSICIANS_OF_RECORD // (0008,1048)
                | tags::PERFORMING_PHYSICIAN_NAME // (0008,1050)
                | tags::NAME_OF_PHYSICIANS_READING_STUDY // (0008,1060)
                | tags::OPERATORS_NAME // (0008,1070)
                | tags::STUDY_DESCRIPTION // (0008,1030) - often contains patient names in practice
            )
        } else {
            false
        }
    }

    /// In-memory de-identification filter purging all DICOM patient/institution tags.
    /// Operates directly on the in-memory dataset without disk writes.
    pub fn sanitize_object(obj: &mut InMemDicomObject) -> usize {
        let mut purged_count = 0;
        let mut tags_to_remove = Vec::new();

        for elem in obj.iter() {
            if Self::is_phi_tag(elem.tag()) {
                tags_to_remove.push(elem.tag());
            }
        }

        for tag in tags_to_remove {
            obj.remove_element(tag);
            purged_count += 1;
        }

        purged_count
    }

    /// Sanitizes a loaded DefaultDicomObject in memory.
    pub fn sanitize_file_object(obj: &mut DefaultDicomObject) -> usize {
        Self::sanitize_object(obj)
    }

    /// Verifies that a DICOM object is completely clean of any PHI tags.
    /// Returns Ok(()) if clean, or Err with the detected PHI tag description.
    pub fn verify_quarantine(obj: &InMemDicomObject) -> Result<(), PrivacyError> {
        for elem in obj.iter() {
            let tag = elem.tag();
            if Self::is_phi_tag(tag) {
                return Err(PrivacyError::PhiDetected(format!(
                    "Tag ({:04X},{:04X}) found in quarantined memory",
                    tag.0, tag.1
                )));
            }
        }
        Ok(())
    }
}

/// Optical Allow-List Validator for cloud inference requests.
pub struct OpticalAllowList;

impl OpticalAllowList {
    /// Validates an inference payload against the in-memory optical allow-list:
    /// - Visual patch: allowed if <= 1024x1024
    /// - Clinician query: allowed
    /// - Magnification MPP: allowed
    /// - Macro/label/barcode: forbidden
    /// - PHI demographic tags: forbidden
    pub fn validate_patch_dimensions(width: u32, height: u32) -> Result<(), PrivacyError> {
        if width > 1024 || height > 1024 {
            return Err(PrivacyError::PatchExceedsDimensionLimit(width, height));
        }
        Ok(())
    }

    pub fn validate_resource_name(name: &str) -> Result<(), PrivacyError> {
        let lower = name.to_ascii_lowercase();
        if lower.contains("label")
            || lower.contains("macro")
            || lower.contains("barcode")
            || lower.contains("overview")
        {
            return Err(PrivacyError::MacroOverviewForbidden);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_core::DataElement;
    use dicom_core::VR;

    #[test]
    fn test_is_phi_tag_patient_group() {
        assert!(DicomSanitizer::is_phi_tag(tags::PATIENT_NAME));
        assert!(DicomSanitizer::is_phi_tag(tags::PATIENT_ID));
        assert!(DicomSanitizer::is_phi_tag(tags::PATIENT_BIRTH_DATE));
        assert!(DicomSanitizer::is_phi_tag(tags::PATIENT_SEX));
        assert!(DicomSanitizer::is_phi_tag(tags::PATIENT_AGE));
        assert!(DicomSanitizer::is_phi_tag(tags::PATIENT_WEIGHT));
        // Any 0x0010 tag
        assert!(DicomSanitizer::is_phi_tag(Tag(0x0010, 0x1234)));
    }

    #[test]
    fn test_is_phi_tag_institution_group() {
        assert!(DicomSanitizer::is_phi_tag(tags::INSTITUTION_NAME));
        assert!(DicomSanitizer::is_phi_tag(tags::INSTITUTION_ADDRESS));
        assert!(DicomSanitizer::is_phi_tag(tags::ACCESSION_NUMBER));
        assert!(DicomSanitizer::is_phi_tag(tags::REFERRING_PHYSICIAN_NAME));
        assert!(DicomSanitizer::is_phi_tag(tags::PERFORMING_PHYSICIAN_NAME));
    }

    #[test]
    fn test_non_phi_geometric_tags_preserved() {
        assert!(!DicomSanitizer::is_phi_tag(tags::PIXEL_SPACING));
        assert!(!DicomSanitizer::is_phi_tag(tags::SLICE_THICKNESS));
        assert!(!DicomSanitizer::is_phi_tag(tags::RESCALE_SLOPE));
        assert!(!DicomSanitizer::is_phi_tag(tags::RESCALE_INTERCEPT));
        assert!(!DicomSanitizer::is_phi_tag(tags::WINDOW_CENTER));
        assert!(!DicomSanitizer::is_phi_tag(tags::WINDOW_WIDTH));
        assert!(!DicomSanitizer::is_phi_tag(tags::ROWS));
        assert!(!DicomSanitizer::is_phi_tag(tags::COLUMNS));
    }

    #[test]
    fn test_in_memory_purging_of_phi_tags() {
        let mut obj = InMemDicomObject::new_empty();

        // Populate patient demographic & institution PHI
        obj.put(DataElement::new(tags::PATIENT_NAME, VR::PN, "DOE^JOHN"));
        obj.put(DataElement::new(tags::PATIENT_ID, VR::LO, "MRN-12345678"));
        obj.put(DataElement::new(tags::PATIENT_BIRTH_DATE, VR::DA, "19800101"));
        obj.put(DataElement::new(tags::INSTITUTION_NAME, VR::LO, "Metropolitan Hospital"));
        obj.put(DataElement::new(tags::ACCESSION_NUMBER, VR::SH, "ACC-987654"));

        // Populate non-PHI geometric tags
        obj.put(DataElement::new(tags::ROWS, VR::IS, "512"));
        obj.put(DataElement::new(tags::COLUMNS, VR::IS, "512"));

        // Before sanitization: PHI detected
        assert!(DicomSanitizer::verify_quarantine(&obj).is_err());
        assert!(obj.element(tags::PATIENT_NAME).is_ok());
        assert!(obj.element(tags::PATIENT_ID).is_ok());

        // Perform in-memory purge
        let purged = DicomSanitizer::sanitize_object(&mut obj);
        assert_eq!(purged, 5);

        // After sanitization: Zero PHI
        assert!(DicomSanitizer::verify_quarantine(&obj).is_ok());
        assert!(obj.element(tags::PATIENT_NAME).is_err());
        assert!(obj.element(tags::PATIENT_ID).is_err());
        assert!(obj.element(tags::PATIENT_BIRTH_DATE).is_err());
        assert!(obj.element(tags::INSTITUTION_NAME).is_err());
        assert!(obj.element(tags::ACCESSION_NUMBER).is_err());

        // Geometric tags remain untouched
        assert!(obj.element(tags::ROWS).is_ok());
        assert!(obj.element(tags::COLUMNS).is_ok());
    }

    #[test]
    fn test_optical_allow_list_validation() {
        assert!(OpticalAllowList::validate_patch_dimensions(1024, 1024).is_ok());
        assert!(OpticalAllowList::validate_patch_dimensions(512, 512).is_ok());
        assert!(OpticalAllowList::validate_patch_dimensions(1025, 512).is_err());
        assert!(OpticalAllowList::validate_patch_dimensions(512, 1200).is_err());

        assert!(OpticalAllowList::validate_resource_name("tissue_roi.png").is_ok());
        assert!(OpticalAllowList::validate_resource_name("slide_label").is_err());
        assert!(OpticalAllowList::validate_resource_name("LABEL_IMAGE.PNG").is_err());
        assert!(OpticalAllowList::validate_resource_name("macro_camera.jpg").is_err());
        assert!(OpticalAllowList::validate_resource_name("OVERVIEW_FRAME").is_err());
        assert!(OpticalAllowList::validate_resource_name("barcode_scan").is_err());
    }

    #[test]
    fn test_empty_dataset_quarantine() {
        let empty_obj = InMemDicomObject::new_empty();
        assert!(DicomSanitizer::verify_quarantine(&empty_obj).is_ok());
        assert_eq!(DicomSanitizer::sanitize_object(&mut InMemDicomObject::new_empty()), 0);
    }

    #[test]
    fn test_all_group_0010_tags_detected_and_purged() {
        let mut obj = InMemDicomObject::new_empty();
        // Arbitrary proprietary or extended group 0x0010 tags
        obj.put(DataElement::new(Tag(0x0010, 0x0040), VR::CS, "M")); // PatientSex
        obj.put(DataElement::new(Tag(0x0010, 0x1010), VR::AS, "045Y")); // PatientAge
        obj.put(DataElement::new(Tag(0x0010, 0x1030), VR::DS, "70.5")); // PatientWeight
        obj.put(DataElement::new(Tag(0x0010, 0x21B0), VR::LT, "Medical history")); // Additional Patient History

        assert!(DicomSanitizer::verify_quarantine(&obj).is_err());
        let purged = DicomSanitizer::sanitize_object(&mut obj);
        assert_eq!(purged, 4);
        assert!(DicomSanitizer::verify_quarantine(&obj).is_ok());
    }
}
