# Specification: Biomedical Literature & PDF Reader

**Document Identifier**: `SPEC-LAMINA-04-READER`  
**System Title**: Embedded Literature Webview & Clinical Context Extractor  
**Revision**: `0.1.0-draft`  
**Status**: Rough Draft (To Revisit Post-Implementation)  
**Parent Document**: `SPEC-LAMINA-00-MASTER`  

---

## 1. Objective & Scope
Specifies the native Webview literature container enabling clinicians to inspect PubMed abstracts, peer-reviewed journal articles, and clinical guideline PDFs side-by-side with imaging viewports, with bidirectional excerpt extraction into diagnostic queries.

---

## 2. Webview Architecture

- **Engine**: Native Webview (WKWebView on macOS, WebView2 on Windows) isolated in a dedicated dockview tab panel.
- **PDF Rendering**: Leverages native Webview PDF rendering engines; requires zero external C/C++ PDF libraries (Poppler, PDFium) or heavy rendering dependencies.
- **Navigation Safety**: Restricts navigation to HTTPS endpoints and local file references (`file://` or custom application asset URLs).

```text
┌────────────────────────────────────────────────────────┐
│                   LITERATURE PANEL                     │
├────────────────────────────────────────────────────────┤
│ Embedded Webview Window                                │
│ ├── Render: PubMed, Nature, NEJM, local PDF protocols   │
│ └── Injected Bridge (`window.__laminaReaderBridge`)     │
│     ├── Selection Listener (`mouseup`, `selectionchange`)│
│     └── Excerpt Dispatcher                             │
└───────────────────────────┬────────────────────────────┘
                            │ Extracted excerpt & DOI
                            ▼
┌────────────────────────────────────────────────────────┐
│           DIAGNOSTIC QUERY CARD (Opt + Space)          │
│ Pinned context: [Text excerpt, PMID/DOI citation link] │
└────────────────────────────────────────────────────────┘
```

---

## 3. Bidirectional Bridge Contract

### 3.1 Injected JavaScript Bridge
```javascript
window.__laminaReaderBridge = {
  getSelectedExcerpt: function() {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return null;
    return {
      text: selection.toString().trim(),
      url: window.location.href,
      title: document.title,
    };
  },
  scrollToAnchor: function(anchorText) {
    // Searches DOM and scrolls matching citation/paragraph into view
    const iterator = document.createNodeIterator(document.body, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = iterator.nextNode())) {
      if (node.nodeValue.includes(anchorText)) {
        node.parentElement.scrollIntoView({ behavior: 'smooth', block: 'center' });
        break;
      }
    }
  }
};
```

### 3.2 Rust IPC Commands
```rust
#[tauri::command]
pub async fn reader_extract_selection(
    window: tauri::WebviewWindow,
) -> Result<Option<ReaderExcerpt>, String>;

#[tauri::command]
pub async fn reader_navigate(
    window: tauri::WebviewWindow,
    url: String,
) -> Result<(), String>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReaderExcerpt {
    pub text: String,
    pub source_url: String,
    pub document_title: String,
    pub pmid_or_doi: Option<String>,
}
```

---

## 4. Verification Criteria
1. Loads remote journal articles (e.g., PubMed) and local PDFs with full formatting and text selection.
2. `reader_extract_selection` reliably returns clean plain-text excerpts without DOM markup corruption.
3. Memory consumption remains isolated within the dedicated Webview process without impacting the $60\text{ FPS}$ WSI canvas.
