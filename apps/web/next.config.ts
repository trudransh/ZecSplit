import type { NextConfig } from "next";

// Static export: the UI is plain files served next to the local engine (ADR-002), no Next server.
const config: NextConfig = {
  output: "export",
  // Each page as a folder with index.html, so any static file server finds /swap.
  trailingSlash: true,
  images: { unoptimized: true },
};

export default config;
