/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_GOOGLE_MAPS_API_KEY: string
  readonly VITE_DISTRI_API_URL: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}