import { useNavigate } from 'react-router-dom'
import { useEffect } from 'react'
import { Button } from '@/components/ui/button'
import { CheckCircle } from 'lucide-react'

export default function PaymentSuccess() {
  const navigate = useNavigate()

  useEffect(() => {
    // Auto redirect after 5 seconds
    const timer = setTimeout(() => {
      navigate('/home/menu/account')
    }, 5000)

    return () => clearTimeout(timer)
  }, [navigate])

  return (
    <div className="min-h-screen bg-gradient-to-br from-green-50 via-emerald-50 to-teal-50 dark:from-green-900/20 dark:via-emerald-900/20 dark:to-teal-900/20 flex items-center justify-center">
      <div className="max-w-md mx-auto text-center p-8">
        <div className="mb-6">
          <div className="mx-auto w-16 h-16 bg-gradient-to-r from-green-500 to-emerald-500 rounded-full flex items-center justify-center mb-4">
            <CheckCircle className="h-8 w-8 text-white" />
          </div>
          <h1 className="text-2xl sm:text-3xl font-bold text-slate-900 dark:text-white mb-2">
            Payment Successful!
          </h1>
          <p className="text-slate-600 dark:text-slate-400">
            Your payment has been processed successfully. You will be redirected to your account shortly.
          </p>
        </div>

        <div className="space-y-4">
          <Button
            onClick={() => navigate('/home/menu/account')}
            className="bg-gradient-to-r from-green-600 to-emerald-600 hover:from-green-700 hover:to-emerald-700 w-full"
          >
            Go to Account
          </Button>
          <Button
            variant="outline"
            onClick={() => navigate('/home')}
            className="w-full"
          >
            Back to Home
          </Button>
        </div>
      </div>
    </div>
  )
}