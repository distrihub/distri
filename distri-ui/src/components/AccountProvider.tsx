'use client'

import { createContext, useContext, useEffect, useState } from 'react'
import { useInitialization } from '@/components/TokenProvider'
import { BACKEND_URL } from '@/constants'

export interface AccountInfo {
  email: string
  subscription_tier: string
  subscription_status: string
  current_period_start: string
  current_period_end: string
  cancelled_at: string | null
  cancel_at: string | null
  cancel_at_period_end: boolean
  current_usage: number
  monthly_credits: number
  picture: string | null
  scope: string | null
  missing_scopes: string[]
}

export enum SubscriptionTier {
  Free = "Free",
  Pro = "Professional",
  Team = "Team",
}

const AccountContext = createContext<{
  accountInfo: AccountInfo | null
  loading: boolean,
  processing: boolean,
  error: string | null
  handleStripeSession: (e: any) => void
}>({
  accountInfo: null,
  loading: true,
  processing: false,
  error: null,
  handleStripeSession: () => { },
})

export const pricingTiers = [
  {
    tier: SubscriptionTier.Free,
    price: "0",
    description: "Great for getting started",
    features: ["50 credits free per month", "Access to all features", "Try for free"],
    cta: "Choose Free",
    ctaLink: "#",
  },
  {
    tier: SubscriptionTier.Pro,
    price: "9.99 USD",
    description: "Perfect for individual power users",
    features: ["10000 credits per month", "Email Support"],
    cta: "Upgrade to Pro",
    ctaLink: "#",
  },
  {
    tier: SubscriptionTier.Team,
    price: "99.99 USD",
    description: "Best for teams and businesses",
    features: ["250000 credits per month", "Unlimited Users", "Slack Connect"],
    cta: "Contact Us",
    ctaLink: "mailto:hello@blinksheets.xyz",
  },
]

export function AccountProvider({ children }: { children: React.ReactNode }) {
  const [accountInfo, setAccountInfo] = useState<AccountInfo | null>(null)
  const [loading, setLoading] = useState(true)
  const [processing, setProcessing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { token } = useInitialization()


  const handleStripeSession = async (e: any) => {
    e.preventDefault()
    setProcessing(true)
    const returnUrl = `${window.location.origin}/home/menu/account`

    try {
      const endpoint = accountInfo?.subscription_tier !== SubscriptionTier.Free
        ? '/api/stripe/create-portal-session'
        : '/api/stripe/create-checkout-session'

      const response = await fetch(`${BACKEND_URL}${endpoint}`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${token}`,
        },
        body: JSON.stringify({
          return_url: returnUrl
        })
      })

      if (!response.ok) {
        setError("Failed to create session")
        setProcessing(false)
        return
      }

      const { url } = await response.json()

      window.location.href = url
      setProcessing(false)
    } catch (error) {
      console.error('Error redirecting to Stripe:', error)
      setError("Error redirecting to Checkout")
      setProcessing(false)
    }
  }
  useEffect(() => {
    if (!token) {
      setLoading(false)
      return
    }

    fetch(`${BACKEND_URL}/api/account`, {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${token}`
      }
    })
      .then((res) => {
        if (res.status > 300) {
          setAccountInfo(null)
          setError("Something went wrong. Please refresh the page. If the problem persists, reach out to hello@blinksheets.xyz")
          setLoading(false)
          return
        }
        return res.json()
      })
      .then((data) => {
        setAccountInfo(data)
        setLoading(false)
      })
      .catch((err) => {
        setAccountInfo(null)
        setError(err.message)
        setLoading(false)
      })
  }, [token])

  return (
    <AccountContext.Provider value={{ accountInfo, loading, error, handleStripeSession, processing }}>
      {children}
    </AccountContext.Provider>
  )
}

export const useAccount = () => useContext(AccountContext) 