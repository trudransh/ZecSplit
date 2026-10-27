import { Pond } from "./pond";

const REPO = "https://github.com/Raveesh1007/Splitzec";
const DOCS = "https://zecsplit.gitbook.io/zecsplit-docs";

/** The koi pond with a link bar, shared by every page. */
export function SiteFooter() {
  return (
    <footer className="footer">
      <Pond className="pond" />
      <div className="pond-bar" data-koi-avoid>
        <span>© 2026 ZecSplit · MIT or Apache-2.0</span>
        <nav aria-label="Footer">
          <a href="/#how">How it works</a>
          <a href="/#privacy">Privacy</a>
          <a href={DOCS} target="_blank" rel="noreferrer">
            Docs
          </a>
          <a href={REPO} target="_blank" rel="noreferrer">
            GitHub
          </a>
        </nav>
        <span>Testnet only · Unaudited</span>
      </div>
    </footer>
  );
}
