"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Logo } from "./logo";
import { ThemeToggle } from "./theme-toggle";

const LINKS = [
  ["/#zcash", "Why Zcash"],
  ["/#safety", "Safety"],
  ["/#privacy", "Privacy"],
  ["/#how", "How it works"],
];

export function Nav({ docs }: { docs: string }) {
  const [scrolled, setScrolled] = useState(false);

  useEffect(() => {
    const on = () => setScrolled(window.scrollY > 24);
    on();
    window.addEventListener("scroll", on, { passive: true });
    return () => window.removeEventListener("scroll", on);
  }, []);

  return (
    <header className={scrolled ? "nav scrolled" : "nav"}>
      <div className="nav-bar">
        <Link href="/" className="brand" aria-label="ZecSplit home">
          <Logo /> ZecSplit
        </Link>
        <nav className="nav-links" aria-label="Sections">
          {LINKS.map(([href, label]) => (
            <a key={href} href={href}>
              {label}
            </a>
          ))}
          <a href={docs} target="_blank" rel="noreferrer">
            Docs
          </a>
        </nav>
        <ThemeToggle />
        <Link href="/swap" className="btn btn-dark btn-sm">
          Start swap <span className="arrow">→</span>
        </Link>
      </div>
    </header>
  );
}
