import { useNavigate } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Header } from '@/components/ui/header'
import { Book, MessageCircle, Mail, ArrowLeft } from 'lucide-react'

export default function HelpPage() {
  const navigate = useNavigate()

  return (
    <div className="h-full flex flex-col">
      <Header
        title="Help & Support"
        subtitle="Get help and learn how to use BlinkSheets"
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
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center space-x-2">
                  <Book className="h-5 w-5" />
                  <span>Documentation</span>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-muted-foreground mb-4">
                  Learn how to use BlinkSheets with our comprehensive documentation.
                </p>
                <Button variant="outline" className="w-full">
                  View Docs
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="flex items-center space-x-2">
                  <MessageCircle className="h-5 w-5" />
                  <span>Community</span>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-muted-foreground mb-4">
                  Join our community to get help from other users and share tips.
                </p>
                <Button variant="outline" className="w-full">
                  Join Community
                </Button>
              </CardContent>
            </Card>

            <Card className="md:col-span-2">
              <CardHeader>
                <CardTitle className="flex items-center space-x-2">
                  <Mail className="h-5 w-5" />
                  <span>Contact Support</span>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-muted-foreground mb-4">
                  Need direct help? Contact our support team and we'll get back to you within 24 hours.
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
    </div>
  )
}
