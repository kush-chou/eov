# Specification: Cloud Clinical Intelligence & Verification Guardrail

**Document Identifier**: `SPEC-LAMINA-05-INTELLIGENCE`  
**System Title**: Two-Tier Clinical Decision Support Pipeline  
**Revision**: `0.1.0-draft`  
**Status**: Rough Draft (To Revisit Post-Implementation)  
**Parent Document**: `SPEC-LAMINA-00-MASTER`  

---

## 1. Objective & Scope
Specifies the cloud inference client and streaming protocol connecting the desktop workstation to the two-tier clinical intelligence pipeline: deep grounded synthesis (Google Gemini Interactions API) and rapid decision calibration (Microsoft-Decision-1 API).

---

## 2. Intelligence Topology

```text
Clinician Query + De-identified Patch (PNG <= 1024x1024)
                       │
                       ▼ (HTTP POST /v1/query)
┌─────────────────────────────────────────────────────────────┐
│                    LAMINA CLOUD GATEWAY                     │
├──────────────────────────────┬──────────────────────────────┤
│ Tier 2: Grounded Synthesis   │ Tier 1: Verification Guardrail
│ Google Interactions API      │ Microsoft-Decision-1 API     │
│ (Gemini 3.8 Flash / Agent)   │ (Calibrated Decision Model)  │
│ ├── PubMed Search Grounding  │ ├── Citation Faithfulness    │
│ ├── URL Context Ingestion    │ ├── Differential Calibration │
│ └── Sandboxed Execution      │ └── Latency: < 100 ms        │
└──────────────────────────────┴──────────────────────────────┘
                       │
                       ▼ (Server-Sent Events / SSE)
┌─────────────────────────────────────────────────────────────┐
│ DESKTOP CLIENT: Real-Time Stream Consumer                   │
│ ├── Streamed Diagnostic Markdown Tokens                     │
│ ├── Interactive Citation Chips (`[1]`)                      │
│ └── Verified Confidence Badge                               │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Tier 2: Google Gemini Interactions API Client

- **Endpoint**: `https://generativelanguage.googleapis.com/v1beta/interactions`
- **Model / Agent**: `agent="antigravity-preview-09-2026"` or `model="gemini-3.8-flash"`.
- **State Invariants**:
  - `previous_interaction_id`: Tracks reasoning context across multi-turn conversations.
  - `environment_id`: Binds to the remote sandbox environment when code execution is active.
- **Native Tools**:
  - `google_search`: Grounding across peer-reviewed literature.
  - `url_context`: Ingests PubMed Central full-text papers via direct HTTPS links.
  - `code_execution`: Isolated remote Linux execution for diagnostic and dosage calculations.

### Payload Schema
```json
{
  "agent": "antigravity-preview-09-2026",
  "environment": "remote",
  "previous_interaction_id": null,
  "input": [
    {
      "type": "text",
      "text": "Analyze this 20x H&E patch from a lung biopsy. Detail the differential diagnosis, recommended IHC panel, and evidence citations."
    },
    {
      "type": "image",
      "mime_type": "image/png",
      "data": "<BASE64_ENCODED_PATCH>"
    }
  ]
}
```

---

## 4. Tier 1: Verification Guardrail (Microsoft-Decision-1)

- **Role**: Validates citation authenticity and scores the consistency of differential diagnoses before presentation.
- **Payload**: Extracts cited claims and corresponding literature chunks; returns a calibrated confidence score $[0.0, 1.0]$.
- **Latency Budget**: Evaluates in $\le 100\text{ ms}$.

---

## 5. Desktop Streaming Contract (SSE)

The Tauri backend consumes the gateway's SSE stream and relays normalized events to the Webview UI:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum DiagnosticStreamEvent {
    #[serde(rename = "token")]
    TokenDelta { text: String },
    
    #[serde(rename = "tool_call")]
    ToolCallStarted { tool_name: String, query: String },
    
    #[serde(rename = "citation")]
    CitationDiscovered { index: u32, title: String, pmid: String, url: String },
    
    #[serde(rename = "verification")]
    VerificationScore { confidence: f32, verified: bool },
    
    #[serde(rename = "done")]
    Completed { interaction_id: String },
    
    #[serde(rename = "error")]
    Failed { code: String, message: String },
}
```

---

## 6. Verification Criteria
1. Diagnostic query stream begins delivering first tokens to the UI in $< 1.2\text{ seconds}$.
2. Citation chips render with valid, clickable PubMed links.
3. Network disconnection or user cancellation (`Esc`) cleanly aborts the active SSE stream without client memory leaks.
