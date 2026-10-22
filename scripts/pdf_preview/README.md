# First working PDF search preview for Akula

This is the smallest manual vertical slice: **one trusted text PDF → a Markdown
search projection in an Akula vault → the existing Akula full-text search**.
The PDF original remains byte-identical beside its `.md` preview; the preview
is explicitly marked `Needs review`.

## Run on Windows

Prerequisites: Python 3, a local vault already opened in Akula, and either the
existing Shiva CLI or Poppler's `pdftotext` executable on your machine.

From a PowerShell window with the script downloaded:

```powershell
py .\import_pdf.py --pdf "C:\Docs\sample.pdf" --vault "C:\MyNotes" --engine shiva --converter "C:\tools\shiva.exe" --trusted-local-only
```

If you already have Poppler instead of the Shiva CLI:

```powershell
py .\import_pdf.py --pdf "C:\Docs\sample.pdf" --vault "C:\MyNotes" --engine poppler --converter "C:\tools\pdftotext.exe" --trusted-local-only
```

Open Akula's existing search and type a distinctive word from the PDF. The
result appears as a Markdown note in `PDF Search Preview/`. The PDF original
is next to it, and its path is recorded inside the note. Whether Akula's
installed Windows UI opens the `.pdf` link directly is *not yet verified*.

## Important boundaries

- **Use only PDFs you created or explicitly trust.** Both conversion backends
  run with your OS user permissions and are NOT the confined Shiva worker.
- Maximum 2 MiB PDF and 2 MiB extracted text, 45-second timeout. No OCR.
- No overwrites, no data sent over the network, no Akula binary modifications.
- Text is always `Needs review`; correct glyph-to-Unicode mapping is not
  guaranteed. This is not automatic production PDF ingestion.
- The installed Akula/File Gate integration, automatic PDF indexing, permission
  revocation, and direct result-to-PDF navigation are separate release gates.

## What the CI proves

A path-scoped GitHub Actions workflow generates a real PDF, extracts it via
Poppler, imports the resulting note, and exercises Akula Core's actual
`IndexSynchronizer` + Tantivy search for the PDF-only `HELLO` token. This is
an honest first functional scenario without claiming installed Windows E2E.