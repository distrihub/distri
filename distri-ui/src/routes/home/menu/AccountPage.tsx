import { useNavigate } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Header } from '@/components/ui/header'
import { ArrowLeft } from 'lucide-react'
import { SubscriptionTier, useAccount } from '@/components/AccountProvider'
import { format } from 'date-fns'

export default function AccountPage() {
  const navigate = useNavigate()

  return (
    <div className="h-full flex flex-col">
      <Header
        title="Account Settings"
        subtitle="Manage your account and billing preferences"
        rightElement={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => navigate('/home')}
            className="h-8 px-3"
          >
            <ArrowLeft className="h-4 w-4 mr-2" />
            Back to Home
          </Button>
        }
      />

      <div className="flex-1 overflow-auto p-6">
        <div className="max-w-2xl">
          <AccountPageContent />
        </div>
      </div>
    </div>
  )
}

export const AccountPageContent = () => {
  const { accountInfo, loading, handleStripeSession, error, processing } = useAccount()

  if (loading) {
    return (
      <Card>
        <CardContent className="p-6">
          <div className="flex items-center justify-center">
            <div className="text-muted-foreground">Loading...</div>
          </div>
        </CardContent>
      </Card>
    )
  }

  if (error) {
    return (
      <Card>
        <CardContent className="p-6">
          <div className="text-destructive">{error}</div>
        </CardContent>
      </Card>
    )
  }

  if (!accountInfo) {
    return (
      <Card>
        <CardContent className="p-6">
          <div className="text-muted-foreground">No account information available</div>
        </CardContent>
      </Card>
    )
  }

  const usagePercentage = (accountInfo.current_usage / accountInfo.monthly_credits) * 100

  return (
    <Card>
      <CardHeader>
        <CardTitle>Account Details</CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div className="space-y-4">
            <div>
              <h3 className="text-sm font-medium mb-1">Email</h3>
              <p className="text-sm text-muted-foreground">{accountInfo.email}</p>
            </div>

            <div>
              <h3 className="text-sm font-medium mb-1">Subscription</h3>
              <div className="space-y-2">
                <p className="text-sm text-muted-foreground">
                  {accountInfo.subscription_tier} Plan
                </p>
                <div className="flex flex-wrap gap-2">
                  <span className="px-2 py-0.5 text-xs rounded-full bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-100">
                    {accountInfo.subscription_status}
                  </span>
                  {accountInfo.cancel_at_period_end && (
                    <span className="px-2 py-0.5 text-xs text-center rounded-full bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-100">
                      Cancels at period end
                    </span>
                  )}
                  {!accountInfo.cancel_at_period_end && accountInfo.cancel_at && (
                    <span className="px-2 py-0.5 text-xs text-center rounded-full bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-100">
                      Cancels on {accountInfo.cancel_at ? format(new Date(accountInfo.cancel_at), 'MMM d, yyyy') : 'N/A'}
                    </span>
                  )}
                </div>
              </div>
            </div>
          </div>

          <div className="space-y-4">
            <div>
              <h3 className="text-sm font-medium mb-1">Billing Period</h3>
              <p className="text-sm text-muted-foreground">
                {accountInfo.current_period_start ? format(new Date(accountInfo.current_period_start), 'MMM d, yyyy') : 'N/A'} -{' '}
                {accountInfo.current_period_end ? format(new Date(accountInfo.current_period_end), 'MMM d, yyyy') : 'N/A'}
              </p>
            </div>

            <div>
              <h3 className="text-sm font-medium mb-2">Credits Usage</h3>
              <div className="bg-secondary h-2 rounded-full overflow-hidden mb-2">
                <div
                  className="bg-primary h-full rounded-full transition-all duration-300"
                  style={{ width: `${Math.min(usagePercentage, 100)}%` }}
                />
              </div>
              <p className="text-sm text-muted-foreground">
                {accountInfo.current_usage} of {accountInfo.monthly_credits} credits used
              </p>
            </div>
          </div>
        </div>

        <div className="pt-4 border-t">
          <div className="flex items-center justify-between mb-4">
            <div>
              <h3 className="text-sm font-medium">Subscription Management</h3>
              <p className="text-sm text-muted-foreground">
                {accountInfo.subscription_tier !== SubscriptionTier.Free
                  ? "Manage your subscription and billing"
                  : "Upgrade your plan to get more features"}
              </p>
            </div>
            <Button variant="link" asChild>
              <a href="/home/menu/account/pricing">View Plans</a>
            </Button>
          </div>

          {error && <div className="text-destructive mb-4 text-sm">{error}</div>}

          <Button
            className="w-full"
            disabled={processing}
            onClick={(e) => handleStripeSession(e)}
          >
            {processing ? "Processing..." : accountInfo.subscription_tier !== SubscriptionTier.Free ? "Manage Subscription" : "Upgrade Plan"}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
