'use client'

import { createContext, useCallback, useContext, useEffect, useState } from 'react'
import { BACKEND_URL } from '@/constants'
import { DeviceProfile } from '@/types'

interface SessionContextValue {
  sessionId: string | null
  setSessionId: (value: string | null) => void
  device: DeviceProfile | null
  loading: boolean
  error: string | null
  refreshDevice: () => Promise<void>
}

const SessionContext = createContext<SessionContextValue | undefined>(undefined)
const SESSION_STORAGE_KEY = 'distri_session_id'

export function SessionProvider({ children }: { children: React.ReactNode }) {
  const [sessionId, setSessionIdState] = useState<string | null>(() => {
    if (typeof window === 'undefined') {
      return null
    }
    try {
      return window.localStorage.getItem(SESSION_STORAGE_KEY)
    } catch (err) {
      console.warn('Failed to read session id from storage', err)
      return null
    }
  })
  const [device, setDevice] = useState<DeviceProfile | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const setSessionId = useCallback((value: string | null) => {
    if (typeof window !== 'undefined') {
      try {
        if (value) {
          window.localStorage.setItem(SESSION_STORAGE_KEY, value)
        } else {
          window.localStorage.removeItem(SESSION_STORAGE_KEY)
        }
      } catch (err) {
        console.warn('Failed to persist session id', err)
        setError('Unable to save session')
      }
    }
    setSessionIdState(value)
  }, [])

  const refreshDevice = useCallback(async () => {
    if (typeof window === 'undefined') {
      setLoading(false)
      return
    }
    setLoading(true)
    try {
      const res = await fetch(`${BACKEND_URL}/v1/device`)
      if (!res.ok) {
        throw new Error(`Failed to fetch device metadata: ${res.status}`)
      }
      const payload: DeviceProfile = await res.json()
      setDevice(payload)
      setError(null)
    } catch (err) {
      console.warn('Failed to load device profile', err)
      setError('Unable to load device profile')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    refreshDevice()
  }, [refreshDevice])

  useEffect(() => {
    if (typeof window === 'undefined') {
      return () => { }
    }
    const listener = (event: StorageEvent) => {
      if (event.key === SESSION_STORAGE_KEY) {
        setSessionIdState(event.newValue)
      }
    }

    window.addEventListener('storage', listener)
    return () => window.removeEventListener('storage', listener)
  }, [])

  return (
    <SessionContext.Provider
      value={{
        sessionId,
        setSessionId,
        device,
        loading,
        error,
        refreshDevice,
      }}
    >
      {children}
    </SessionContext.Provider>
  )
}

export const useSession = () => {
  const ctx = useContext(SessionContext)
  if (!ctx) {
    throw new Error('useSession must be used within a SessionProvider')
  }
  return ctx
}
