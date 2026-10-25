import type { NextConfig } from "next";

// Static export: the UI is plain files served next to the local engine (ADR-002), no Next server.
const config: NextConfig = {
  output: "export",
  images: { unoptimized: true },
};

export default config;
