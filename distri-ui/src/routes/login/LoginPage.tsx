import { FormEvent, useEffect, useMemo, useState } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useSession } from '@/components/SessionProvider'
import { DISTRI_CLOUD_URL } from '@/constants'
import { LogIn, Mail } from 'lucide-react'
import { getLoginConfig } from '@/config/login'

const generateSessionId = () => {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `session-${Date.now()}-${Math.random().toString(16).slice(2)}`
}

interface LocationState {
  from?: string
}

export default function LoginPage() {
  const { sessionId, setSessionId, device, loading, error } = useSession()
  const [email, setEmail] = useState('')
  const [submitError, setSubmitError] = useState<string | null>(null)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [logoFailed, setLogoFailed] = useState(false)
  const loginConfig = useMemo(() => getLoginConfig(), [])
  const brandName = loginConfig.brandName || 'Distri'
  const accentColor = loginConfig.accentColor || 'hsl(188, 70%, 45%)'
  const buttonLabel = loginConfig.buttonLabel || 'Continue'
  const emailPlaceholder = loginConfig.emailPlaceholder || 'your@email.com'
  const backgroundColor = loginConfig.backgroundColor || '#000000'
  const termsUrl = loginConfig.termsUrl || 'https://distri.dev/terms'
  const navigate = useNavigate()
  const location = useLocation()
  const redirectTo = useMemo(() => {
    const state = location.state as LocationState | null
    return state?.from || '/home'
  }, [location.state])

  useEffect(() => {
    if (!loading && sessionId) {
      navigate(redirectTo, { replace: true })
    }
  }, [loading, sessionId, navigate, redirectTo])

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const normalizedEmail = email.trim().toLowerCase()
    if (!normalizedEmail) {
      setSubmitError('Please enter your email address')
      return
    }

    const run = async () => {
      setSubmitError(null)
      setIsSubmitting(true)
      const nextSessionId = sessionId || device?.device_id || generateSessionId()

      const payload = {
        email: normalizedEmail,
        session_id: nextSessionId,
        device_id: device?.device_id || nextSessionId,
        device_type: device?.device_type,
        os: device?.os,
        arch: device?.arch,
        hostname: device?.hostname ?? undefined,
        storage_scope: device?.storage_scope,
      }

      try {
        const controller = new AbortController()
        const timeout = setTimeout(() => controller.abort(), 8000)

        const res = await fetch(`${DISTRI_CLOUD_URL}/instrument`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
          signal: controller.signal,
          keepalive: true,
        })

        clearTimeout(timeout)

        if (!res.ok) {
          throw new Error(`Instrumentation failed with status ${res.status}`)
        }

        setSessionId(nextSessionId)
        navigate(redirectTo, { replace: true })
      } catch (err) {
        console.warn('Instrumentation request failed', err)
        setSubmitError('Unable to reach Distri. Please check your connection and try again.')
      } finally {
        setIsSubmitting(false)
      }
    }

    void run()
  }

  return (
    <div
      className="min-h-screen text-white flex items-center justify-center px-6"
      style={{ backgroundColor }}
    >
      <div className="w-full max-w-3xl flex flex-col items-center gap-8">
        <div className="text-center space-y-4">
          <div className="flex flex-col items-center gap-3">
            {loginConfig.logoSrc && !logoFailed ? (
              <img
                src={loginConfig.logoSrc}
                alt={brandName}
                className="h-16 w-auto"
                onError={() => setLogoFailed(true)}
              />
            ) : (
              <h1 className="text-4xl sm:text-5xl font-semibold leading-tight" style={{ color: accentColor }}>
                {brandName}
              </h1>
            )}
          </div>
        </div>

        <form className="w-full max-w-2xl space-y-4" onSubmit={handleSubmit}>
          <div className="relative">
            <span className="absolute inset-y-0 left-3 flex items-center text-gray-400">
              <Mail size={18} />
            </span>
            <Input
              id="email"
              name="email"
              type="email"
              placeholder={emailPlaceholder}
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              disabled={isSubmitting}
              required
              className="pl-10 h-14 text-lg bg-neutral-900 border-neutral-800 text-white placeholder:text-neutral-600"
            />
          </div>

          {(submitError || error) && (
            <div className="rounded-md border border-red-800 bg-red-950/60 px-3 py-2 text-sm text-red-100">
              {submitError || error}
            </div>
          )}

          <Button
            type="submit"
            className="w-full h-14 text-lg text-white"
            size="lg"
            disabled={isSubmitting}
            style={{
              backgroundColor: accentColor,
              borderColor: accentColor,
            }}
          >
            <LogIn className="mr-2" size={18} />
            {isSubmitting ? 'Saving...' : buttonLabel}
          </Button>
        </form>

        {loginConfig.showFooter && (
          <div className="text-center text-xs text-neutral-500 space-y-1">
            <p>
              By using {brandName} you accept our{' '}
              <a
                href={termsUrl}
                target="_blank"
                rel="noreferrer"
                style={{ color: accentColor }}
                className="underline"
              >
                terms
              </a>
              .
            </p>
          </div>
        )}
      </div>
    </div>
  )
}
