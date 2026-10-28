import type { Metadata, Viewport } from "next";
import "@fontsource-variable/host-grotesk";
import "@fontsource-variable/jetbrains-mono";
import "@fontsource/geist-mono/600.css";
import "./globals.css";

export const metadata: Metadata = {
  title: "ZecSplit · Swap shielded ZEC for BTC",
  description:
    "Trustless atomic swaps from shielded ZEC to BTC. No bridge, no custodian, no transparent address. Nobody in the middle can freeze or take your swap.",
  icons: { icon: "/favicon.svg" },
};

export const viewport: Viewport = {
  themeColor: "#f9f9f7",
};

// Runs before paint so a stored dark choice never flashes light first.
const themeScript = `try{document.documentElement.dataset.theme=localStorage.getItem("theme")||"light"}catch(e){}`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" data-theme="light" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
