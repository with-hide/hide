// pdf.js, loaded once when a PDF is first drawn: the PDF viewer (`PdfBody`)
// and Desk's preview tiles (`filePreview.ts`) both ask for it here, so the
// library and its worker are set up in one place.

type Pdfjs = typeof import("pdfjs-dist");

let loading: Promise<Pdfjs> | null = null;

export function loadPdfjs(): Promise<Pdfjs> {
  loading ??= (async () => {
    const pdfjs = await import("pdfjs-dist");
    const worker = await import("pdfjs-dist/build/pdf.worker.min.mjs?url");
    pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
    return pdfjs;
  })();
  return loading;
}
