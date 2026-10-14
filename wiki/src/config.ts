/** Where links to the code and to the wiki sources point. */
export const REPO_URL = "https://github.com/nivalume/vbt";

/**
 * Commit the line numbers in the pages were checked against. Links to code
 * (`crates/…:123`) open this commit so the lines still match after the code
 * moves on. Override with VITE_CODE_REF when re-verifying against newer code.
 */
export const CODE_REF: string = import.meta.env.VITE_CODE_REF ?? "cffa4f13f5ff2821e81fc9ecb759652c23af66f7";

/** Branch used for "edit this page" and for links to non-code repository files. */
export const CONTENT_REF: string = import.meta.env.VITE_CONTENT_REF ?? "main";

export const SITE_TITLE = "phasor 知识库";
