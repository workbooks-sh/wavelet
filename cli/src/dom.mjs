// Inject linkedom's DOMParser into globalThis so the wavelet-runtime
// parser (which uses DOMParser) runs in Node.

import { DOMParser } from "linkedom";

if (typeof globalThis.DOMParser === "undefined") {
  globalThis.DOMParser = DOMParser;
}
