import { useNavigate } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Header } from '@/components/ui/header'
import { CreditCard, Check, ArrowLeft, Mail } from 'lucide-react'

export default function PricingPage() {
  const navigate = useNavigate()

  return (
    <div className="h-full flex flex-col">
      <Header
        title="Choose Your Plan"
        subtitle="Select the plan that best fits your needs"
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
        <div className="max-w-4xl">
          <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-center">
                  <div className="text-lg font-semibold">Free</div>
                  <div className="text-3xl font-bold mt-2">$0<span className="text-sm font-normal">/month</span></div>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <ul className="space-y-2">
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">10 enrichments/month</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">Basic AI models</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">Email support</span>
                  </li>
                </ul>
                <Button variant="outline" className="w-full">
                  Current Plan
                </Button>
              </CardContent>
            </Card>

            <Card className="relative">
              <div className="absolute -top-3 left-1/2 transform -translate-x-1/2">
                <span className="bg-primary text-primary-foreground px-3 py-1 rounded-full text-xs font-medium">
                  Popular
                </span>
              </div>
              <CardHeader>
                <CardTitle className="text-center">
                  <div className="text-lg font-semibold">Pro</div>
                  <div className="text-3xl font-bold mt-2">$29<span className="text-sm font-normal">/month</span></div>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <ul className="space-y-2">
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">1,000 enrichments/month</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">Advanced AI models</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">Priority support</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">Custom enrichments</span>
                  </li>
                </ul>
                <Button className="w-full">
                  <CreditCard className="h-4 w-4 mr-2" />
                  Upgrade to Pro
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-center">
                  <div className="text-lg font-semibold">Enterprise</div>
                  <div className="text-3xl font-bold mt-2">$99<span className="text-sm font-normal">/month</span></div>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <ul className="space-y-2">
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">Unlimited enrichments</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">All AI models</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">24/7 support</span>
                  </li>
                  <li className="flex items-center">
                    <Check className="h-4 w-4 text-green-500 mr-2" />
                    <span className="text-sm">API access</span>
                  </li>
                </ul>
                <Button variant="outline" className="w-full">
                  Contact Sales
                </Button>
              </CardContent>
            </Card>
          </div>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center space-x-2">
                <Mail className="h-5 w-5" />
                <span>Contact Support</span>
              </CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-muted-foreground mb-4">
                Need help choosing the right plan? Contact our support team and we'll help you find the perfect solution for your needs.
              </p>
              <Button>
                <Mail className="h-4 w-4 mr-2" />
                Contact Support
              </Button>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}