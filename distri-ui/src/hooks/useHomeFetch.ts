import { useCallback } from 'react'
import { BACKEND_URL } from '@/constants'
import { useInitialization } from '@/components/TokenProvider'

export function useHomeFetch() {
  const { token } = useInitialization()

  return useCallback(
    (path: string, init?: RequestInit) => {
      const headers = new Headers(init?.headers || undefined)
      if (token) {
        headers.set('Authorization', `Bearer ${token}`)
      }
      return fetch(`${BACKEND_URL}${path}`, {
        ...init,
        headers,
      })
    },
    [token],
  )
}
