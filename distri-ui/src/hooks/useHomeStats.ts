import { useEffect, useState } from 'react'
import { BACKEND_URL } from '@/constants'

export interface HomeStats {
  total_agents?: number
  total_threads?: number
  total_messages?: number
  avg_time_per_run_ms?: number
}

export function useHomeStats() {
  const [stats, setStats] = useState<HomeStats | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      setLoading(true)
      try {
        const res = await fetch(`${BACKEND_URL}/v1/home/stats`)
        if (res.ok) {
          const json = (await res.json()) as HomeStats
          if (!cancelled) {
            setStats(json)
            setError(null)
          }
        } else {
          throw new Error(`Status ${res.status}`)
        }
      } catch (err: any) {
        if (!cancelled) {
          setStats(null)
          setError(err?.message || 'Failed to load stats')
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    load()
    return () => {
      cancelled = true
    }
  }, [])

  return { stats, loading, error }
}


