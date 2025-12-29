import { ConfigurationPanel } from '@distri/react'
import { useEffect, useState } from 'react'

import { BACKEND_URL } from '@/constants'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'

type SettingsMode = 'panel' | 'raw'

const SETTINGS_ENDPOINT = `${BACKEND_URL}/api/v1/settings`

const SettingsView = () => {
  const [mode, setMode] = useState<SettingsMode>('panel')
  const [rawSettings, setRawSettings] = useState('{\n}')
  const [status, setStatus] = useState<string | null>(null)
  const [isLoading, setIsLoading] = useState(false)

  const loadSettings = async () => {
    setIsLoading(true)
    setStatus(null)
    try {
      const response = await fetch(SETTINGS_ENDPOINT)
      if (!response.ok) {
        throw new Error(`Failed to load settings (${response.status})`)
      }
      const payload = await response.json()
      const formatted = JSON.stringify(payload.settings ?? {}, null, 2)
      setRawSettings(formatted)
    } catch (error) {
      setStatus(
        error instanceof Error ? error.message : 'Failed to load settings.'
      )
    } finally {
      setIsLoading(false)
    }
  }

  useEffect(() => {
    loadSettings()
  }, [])

  const handleSave = async () => {
    setStatus(null)
    let parsed: unknown
    try {
      parsed = JSON.parse(rawSettings)
    } catch (error) {
      setStatus(
        error instanceof Error ? error.message : 'Settings JSON is invalid.'
      )
      return
    }

    setIsLoading(true)
    try {
      const response = await fetch(SETTINGS_ENDPOINT, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ settings: parsed }),
      })
      if (!response.ok) {
        throw new Error(`Failed to save settings (${response.status})`)
      }
      setStatus('Settings saved.')
      const payload = await response.json()
      setRawSettings(JSON.stringify(payload.settings ?? {}, null, 2))
    } catch (error) {
      setStatus(
        error instanceof Error ? error.message : 'Failed to save settings.'
      )
    } finally {
      setIsLoading(false)
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-foreground">Settings</h2>
          <p className="text-sm text-muted-foreground">
            Toggle between the guided configuration view and raw JSON settings.
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            type="button"
            variant={mode === 'panel' ? 'default' : 'outline'}
            onClick={() => setMode('panel')}
          >
            Configuration UI
          </Button>
          <Button
            type="button"
            variant={mode === 'raw' ? 'default' : 'outline'}
            onClick={() => setMode('raw')}
          >
            Raw JSON
          </Button>
        </div>
      </div>

      {mode === 'panel' ? (
        <ConfigurationPanel title="Settings" />
      ) : (
        <div className="space-y-4">
          <Textarea
            className="min-h-[320px] font-mono text-sm"
            value={rawSettings}
            onChange={(event) => setRawSettings(event.target.value)}
            spellCheck={false}
          />
          <div className="flex flex-wrap items-center gap-3">
            <Button type="button" onClick={handleSave} disabled={isLoading}>
              Save settings
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={loadSettings}
              disabled={isLoading}
            >
              Reload
            </Button>
            {status ? (
              <span className="text-sm text-muted-foreground">{status}</span>
            ) : null}
          </div>
        </div>
      )}
    </div>
  )
}

export default SettingsView
