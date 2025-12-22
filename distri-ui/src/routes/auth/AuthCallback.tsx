import { useEffect, useState, Suspense } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { BACKEND_URL } from '@/constants'

// Separate the main component logic that uses useSearchParams
function AuthCallbackContent() {
  const [error, setError] = useState<string | null>(null)
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()

  useEffect(() => {
    const handleCallback = async () => {
      // Get all search params
      const searchParamsObject: { [key: string]: string } = {}
      searchParams.forEach((value, key) => {
        searchParamsObject[key] = value
      })

      if (!searchParamsObject.code) {
        setError('No authorization code received')
        return
      }

      try {
        // Create URL with all search params
        const callbackUrl = new URL(`${BACKEND_URL}/auth/google/callback`)

        const body = {
          code: searchParamsObject.code,
          state: searchParamsObject.state,
          redirect_uri: `${window.location.origin}/auth/callback`,
        }

        const response = await fetch(callbackUrl.toString(), {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify(body),
        })

        if (!response.ok) {
          throw new Error('Failed to get access token')
        }

        const data = await response.json()
        const authToken = data.token

        if (authToken) {
          // Store token in localStorage
          localStorage.setItem('auth_token', authToken)
          navigate('/auth/success')
        } else {
          throw new Error('Failed to get token')
        }
      } catch (err) {
        setError('Authentication failed')
      }
    }

    handleCallback()
  }, [searchParams, navigate])

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex flex-col items-center justify-center">
      <div className="flex flex-col items-center gap-8">
        <img
          src="/images/blinksheets.png"
          alt="Blink AI"
          className="h-16 w-auto"
        />
        {error ? (
          <div className="text-red-500">{error}</div>
        ) : (
          <div className="flex flex-col items-center gap-4">
            <div className="loading-spinner"></div>
            <div className="text-gray-600 dark:text-gray-400">
              Completing authentication...
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

// Main component wrapped in Suspense
export default function AuthCallback() {
  return (
    <Suspense fallback={
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex flex-col items-center justify-center">
        <div className="flex flex-col items-center gap-8">
          <img
            src="/images/blinksheets.png"
            alt="Blink AI"
            className="h-16 w-auto"
          />
          <div className="flex flex-col items-center gap-4">
            <div className="loading-spinner"></div>
            <div className="text-gray-600 dark:text-gray-400">
              Loading...
            </div>
          </div>
        </div>
      </div>
    }>
      <AuthCallbackContent />
    </Suspense>
  )
}