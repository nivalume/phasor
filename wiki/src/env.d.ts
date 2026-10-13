/// <reference types="vite/client" />

declare module "virtual:repo-files" {
  /** Repository files under the code and docs roots, listed when the app is built. */
  const files: string[];
  export default files;
}

interface ImportMetaEnv {
  readonly VITE_CODE_REF?: string;
  readonly VITE_CONTENT_REF?: string;
}
