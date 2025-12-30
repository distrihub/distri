'use client'

import { createContext, useCallback, useContext, useEffect, useState } from 'react'

interface TokenContextType {
  token: string | null
  setToken: (token: string | null) => void,
  error: string | null
}

const TokenContext = createContext<TokenContextType | undefined>(undefined)

export function TokenProvider({ children }: { children: React.ReactNode }) {
  const [token, setTokenState] = useState<string | null>(() => {
    if (typeof window === 'undefined') {
      return null
    }
    try {
      return window.localStorage.getItem('auth_token')
    } catch (err) {
      console.warn('Failed to read auth token from storage', err)
      return null
    }
  })
  const [error, setError] = useState<string | null>(null)

  const setToken = useCallback(
    (value: string | null) => {
      if (typeof window !== 'undefined') {
        try {
          if (value) {
            window.localStorage.setItem('auth_token', value)
          } else {
            window.localStorage.removeItem('auth_token')
          }
        } catch (err) {
          console.warn('Failed to persist auth token', err)
          setError('Failed to persist authentication token')
        }
      }
      setTokenState(value)
    },
    [],
  )

  useEffect(() => {
    if (typeof window === 'undefined') {
      return
    }
    const storedToken = window.localStorage.getItem('auth_token')
    if (storedToken && storedToken !== token) {
      setTokenState(storedToken)
    }
  }, [token])

  useEffect(() => {
    if (typeof window === 'undefined') {
      return () => {}
    }
    const listener = (event: StorageEvent) => {
      if (event.key === 'auth_token') {
        setTokenState(event.newValue)
      }
    }
    window.addEventListener('storage', listener)
    return () => window.removeEventListener('storage', listener)
  }, [])

  if (error) {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex flex-col items-center justify-center">
        <div className="flex flex-col items-center gap-8">
          <img
            src="/images/blinksheets.png"
            alt="Blink AI"
            className="h-16 w-auto"
          />
          <p className="text-red-500">{error}</p>
        </div>
      </div>
    )
  }

  return (
    <TokenContext.Provider value={{ token, setToken, error }}>
      {children}
    </TokenContext.Provider>
  )
}

export const useInitialization = () => {
  const context = useContext(TokenContext)
  if (context === undefined) {
    throw new Error('useToken must be used within a TokenProvider')
  }
  return context
}
