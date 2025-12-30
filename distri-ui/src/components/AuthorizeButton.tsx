import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { BACKEND_URL } from '@/constants'

export default function AuthorizeButton() {
  const [isLoading, setIsLoading] = useState(false)

  const redirect_uri = `${window.location.origin}/auth/callback`;
  const handleAuthorize = async () => {
    setIsLoading(true)

    try {
      const response = await fetch(`${BACKEND_URL}/auth/google/url?redirect_uri=${redirect_uri}`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          redirect_uri
        }),
      })

      if (response.ok) {
        const { auth_url } = await response.json()
        console.log(auth_url)
        window.location.href = auth_url
      } else {
        throw new Error('Failed to get authorization URL')
      }
    } catch (error) {
      console.error('Authorization error:', error)
    } finally {
      setIsLoading(false)
    }
  }

  return (
    <Button
      onClick={handleAuthorize}
      disabled={isLoading}
      className="bg-gradient-to-r from-blue-600 to-indigo-600 hover:from-blue-700 hover:to-indigo-700 text-white px-6 py-3 rounded-lg shadow-lg"
    >
      {isLoading ? 'Authorizing...' : 'Authorize with Google'}
    </Button>
  )
}