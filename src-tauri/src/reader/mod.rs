use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Reader excerpt contract matching SPEC-04-READER.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReaderExcerpt {
    pub text: String,
    pub source_url: String,
    pub document_title: String,
    pub pmid_or_doi: Option<String>,
}

/// Clinical document section with anchor for deep-linking from CDS citation chips.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReaderSection {
    pub anchor: String,
    pub heading: String,
    pub content: String,
}

/// Structured medical reference document available in the native reader.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReaderDocument {
    pub id: String,
    pub title: String,
    pub citation: String,
    pub pmid: Option<String>,
    pub doi: Option<String>,
    pub sections: Vec<ReaderSection>,
}

/// State of the embedded Biomedical Literature & PDF Reader.
pub struct ReaderState {
    pub current_selection: Mutex<Option<ReaderExcerpt>>,
    pub active_url: Mutex<String>,
    pub documents: Vec<ReaderDocument>,
}

impl ReaderState {
    pub fn new() -> Self {
        let documents = vec![
            ReaderDocument {
                id: "who-thoracic-5th".to_string(),
                title: "WHO Classification of Thoracic Tumours (5th Edition)".to_string(),
                citation: "World Health Organization Classification of Tumours, 5th Edition, 2021".to_string(),
                pmid: Some("33906806".to_string()),
                doi: Some("10.1016/j.jtho.2021.01.002".to_string()),
                sections: vec![
                    ReaderSection {
                        anchor: "who-ihc-workup".to_string(),
                        heading: "Diagnostic IHC Algorithm for Poorly Differentiated Non-Small Cell Carcinoma".to_string(),
                        content: "In small biopsies and cytology specimens lacking definitive squamous or glandular differentiation, a minimal two-marker panel of TTF-1 and p40 should be employed to conserve diagnostic tissue for molecular testing. TTF-1 positivity supports adenocarcinoma, while diffuse p40 positivity confirms squamous cell carcinoma.".to_string(),
                    },
                    ReaderSection {
                        anchor: "who-adeno-patterns".to_string(),
                        heading: "Adenocarcinoma Histologic Subtypes and Grading".to_string(),
                        content: "Invasive pulmonary adenocarcinoma is classified by predominant histological pattern: lepidic, acinar, papillary, micropapillary, and solid. High-grade patterns (micropapillary and solid >= 20%) are associated with elevated recurrence risk.".to_string(),
                    },
                ],
            },
            ReaderDocument {
                id: "travis-small-biopsies-2011".to_string(),
                title: "Diagnosis of Lung Cancer in Small Biopsies and Cytology".to_string(),
                citation: "Travis WD, et al. J Thorac Oncol. 2011;6(2):244-285.".to_string(),
                pmid: Some("21317737".to_string()),
                doi: Some("10.1097/JTO.0b013e318206a221".to_string()),
                sections: vec![
                    ReaderSection {
                        anchor: "travis-p40-specificity".to_string(),
                        heading: "Superiority of p40 over p63 in Pulmonary Squamous Diagnosis".to_string(),
                        content: "p40 (delta-Np63) offers equivalent sensitivity (~100%) to p63 for pulmonary squamous cell carcinoma with significantly superior specificity (~98% vs ~65-70%), avoiding false-positive diagnoses in adenocarcinomas and large cell neuroendocrine carcinomas.".to_string(),
                    },
                ],
            },
            ReaderDocument {
                id: "cap-iaslc-amp-2018".to_string(),
                title: "Updated Molecular Testing Guideline for Selection of Lung Cancer Patients for TKIs".to_string(),
                citation: "Lindeman NI, et al. Arch Pathol Lab Med. 2018;142(3):321-346.".to_string(),
                pmid: Some("29360357".to_string()),
                doi: Some("10.5858/arpa.2017-0388-CP".to_string()),
                sections: vec![
                    ReaderSection {
                        anchor: "molecular-reflex-panel".to_string(),
                        heading: "Recommended Biomarkers in Advanced Stage Non-Small Cell Lung Carcinoma".to_string(),
                        content: "Multiplex molecular testing for EGFR mutations, ALK rearrangements, ROS1 fusions, and BRAF V600E mutations should be routinely performed on all patients with advanced stage lung adenocarcinoma. Tissue stewardship during the initial histologic diagnosis is critical to ensure sufficient material for next-generation sequencing.".to_string(),
                    },
                ],
            },
        ];

        Self {
            current_selection: Mutex::new(None),
            active_url: Mutex::new("lamina-reader://guidelines/who-thoracic-5th".to_string()),
            documents,
        }
    }

    /// Sets the currently highlighted reader excerpt.
    pub fn set_selection(&self, excerpt: ReaderExcerpt) {
        let mut guard = self.current_selection.lock().unwrap();
        *guard = Some(excerpt);
    }

    /// Retrieves the active selection.
    pub fn get_selection(&self) -> Option<ReaderExcerpt> {
        self.current_selection.lock().unwrap().clone()
    }

    /// Clears the active selection.
    pub fn clear_selection(&self) {
        let mut guard = self.current_selection.lock().unwrap();
        *guard = None;
    }

    /// Validates navigation target URL.
    pub fn validate_url(url: &str) -> Result<(), &'static str> {
        if url.starts_with("https://") || url.starts_with("file://") || url.starts_with("lamina-") {
            Ok(())
        } else {
            Err("Navigation restricted to secure HTTPS, file, or lamina asset protocols")
        }
    }

    /// Sets active URL after validation.
    pub fn set_active_url(&self, url: String) -> Result<(), &'static str> {
        Self::validate_url(&url)?;
        let mut guard = self.active_url.lock().unwrap();
        *guard = url;
        Ok(())
    }

    /// Retrieves active URL.
    pub fn get_active_url(&self) -> String {
        self.active_url.lock().unwrap().clone()
    }

    /// Finds document by ID, title, citation, PMID, or DOI.
    pub fn find_document(&self, query: &str) -> Option<&ReaderDocument> {
        let q = query.to_lowercase();
        self.documents.iter().find(|d| {
            d.id.to_lowercase().contains(&q)
                || d.pmid.as_deref().map(|p| p.to_lowercase().contains(&q)).unwrap_or(false)
                || d.doi.as_deref().map(|doi| doi.to_lowercase().contains(&q)).unwrap_or(false)
                || d.title.to_lowercase().contains(&q)
                || d.citation.to_lowercase().contains(&q)
        })
    }
}

impl Default for ReaderState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reader_state_selection_cycle() {
        let reader = ReaderState::new();
        assert!(reader.get_selection().is_none());

        let excerpt = ReaderExcerpt {
            text: "TTF-1 is positive in 85% of adenocarcinomas".to_string(),
            source_url: "https://pubmed.ncbi.nlm.nih.gov/33906806/".to_string(),
            document_title: "WHO Thoracic Tumours".to_string(),
            pmid_or_doi: Some("33906806".to_string()),
        };

        reader.set_selection(excerpt.clone());
        assert_eq!(reader.get_selection(), Some(excerpt));

        reader.clear_selection();
        assert!(reader.get_selection().is_none());
    }

    #[test]
    fn test_reader_url_validation() {
        assert!(ReaderState::validate_url("https://pubmed.ncbi.nlm.nih.gov/33906806/").is_ok());
        assert!(ReaderState::validate_url("file:///documents/guideline.pdf").is_ok());
        assert!(ReaderState::validate_url("lamina-reader://who-thoracic").is_ok());

        assert!(ReaderState::validate_url("http://insecure-site.com").is_err());
        assert!(ReaderState::validate_url("javascript:alert(1)").is_err());
        assert!(ReaderState::validate_url("data:text/html,<h1>hack</h1>").is_err());
    }

    #[test]
    fn test_reader_document_lookup() {
        let reader = ReaderState::new();
        let doc = reader.find_document("33906806");
        assert!(doc.is_some());
        assert_eq!(doc.unwrap().id, "who-thoracic-5th");

        let doc2 = reader.find_document("travis");
        assert!(doc2.is_some());
        assert_eq!(doc2.unwrap().pmid.as_deref(), Some("21317737"));

        let doc3 = reader.find_document("10.5858/arpa.2017-0388-CP");
        assert!(doc3.is_some());
        assert_eq!(doc3.unwrap().id, "cap-iaslc-amp-2018");

        assert!(reader.find_document("nonexistent").is_none());
    }
}
