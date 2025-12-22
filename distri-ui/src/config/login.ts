export interface LoginConfig {
  logoSrc: string
  brandName: string
  welcomePrefix: string
  accentColor: string
  backgroundColor: string
  termsUrl: string
  buttonLabel: string
  emailPlaceholder: string
  showFooter: boolean
}

const defaultLoginConfig: LoginConfig = {
  logoSrc: '/logo.svg',
  brandName: 'Distri',
  welcomePrefix: 'Welcome to',
  accentColor: 'hsl(188, 70%, 45%)',
  backgroundColor: '#000000',
  termsUrl: 'https://distri.dev/terms',
  buttonLabel: 'Continue',
  emailPlaceholder: 'your@email.com',
  showFooter: true
}

const sanitizeString = (value: string | undefined, fallback: string): string => {
  if (!value) return fallback
  const trimmed = value.trim()
  return trimmed.length === 0 ? fallback : trimmed
}

const toBool = (value: string | undefined): boolean | undefined => {
  if (value === undefined) return undefined
  return value.toLowerCase() === 'true'
}

const envOverrides = (): Partial<LoginConfig> => {
  const env = import.meta.env || {}
  return {
    logoSrc: env.VITE_LOGIN_LOGO_SRC,
    brandName: env.VITE_LOGIN_BRAND_NAME,
    welcomePrefix: env.VITE_LOGIN_WELCOME_PREFIX,
    accentColor: env.VITE_LOGIN_ACCENT_COLOR,
    backgroundColor: env.VITE_LOGIN_BACKGROUND_COLOR,
    termsUrl: env.VITE_LOGIN_TERMS_URL,
    buttonLabel: env.VITE_LOGIN_BUTTON_LABEL,
    emailPlaceholder: env.VITE_LOGIN_EMAIL_PLACEHOLDER,
    showFooter: toBool(env.VITE_LOGIN_SHOW_FOOTER),
  }
}

const runtimeOverrides = (): Partial<LoginConfig> => {
  if (typeof window === 'undefined') return {}
  const runtimeConfig = (window as any).__DISTRI_LOGIN_CONFIG
  if (runtimeConfig && typeof runtimeConfig === 'object') {
    return runtimeConfig as Partial<LoginConfig>
  }
  return {}
}

export const getLoginConfig = (): LoginConfig => {
  const mergedRaw: LoginConfig = {
    ...defaultLoginConfig,
    ...envOverrides(),
    ...runtimeOverrides(),
  }

  const merged: LoginConfig = {
    ...mergedRaw,
    logoSrc: sanitizeString(mergedRaw.logoSrc, defaultLoginConfig.logoSrc),
    brandName: sanitizeString(mergedRaw.brandName, defaultLoginConfig.brandName),
    welcomePrefix: sanitizeString(mergedRaw.welcomePrefix, defaultLoginConfig.welcomePrefix),
    accentColor: sanitizeString(mergedRaw.accentColor, defaultLoginConfig.accentColor),
    backgroundColor: sanitizeString(mergedRaw.backgroundColor, defaultLoginConfig.backgroundColor),
    termsUrl: sanitizeString(mergedRaw.termsUrl, defaultLoginConfig.termsUrl),
    buttonLabel: sanitizeString(mergedRaw.buttonLabel, defaultLoginConfig.buttonLabel),
    emailPlaceholder: sanitizeString(
      mergedRaw.emailPlaceholder,
      defaultLoginConfig.emailPlaceholder,
    ),
    showFooter:
      mergedRaw.showFooter === undefined ? defaultLoginConfig.showFooter : mergedRaw.showFooter,
  }

  return merged
}
