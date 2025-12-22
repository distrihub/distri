import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Play, Loader2 } from 'lucide-react'
import { BACKEND_URL } from '@/constants'

interface WorkflowRunnerProps {
  agent: any
  firstStepType: 'tool' | 'agent' | null
}

export const WorkflowRunner = ({ agent, firstStepType }: WorkflowRunnerProps) => {
  const [isRunning, setIsRunning] = useState(false)
  const [message, setMessage] = useState('')
  const [result, setResult] = useState<any>(null)

  const runWorkflow = async () => {
    setIsRunning(true)
    setResult(null)

    try {
      // Call the a2a API directly
      const response = await fetch(`${BACKEND_URL}/api/v1/agents/${agent.id}/invoke`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          message: firstStepType === 'agent' ? message : 'Execute workflow',
        }),
      })

      if (response.ok) {
        const data = await response.json()
        setResult(data)
      } else {
        const errorText = await response.text()
        setResult({ error: `Failed to run workflow: ${errorText}` })
      }
    } catch (error: any) {
      setResult({ error: `Network error: ${error.message}` })
    } finally {
      setIsRunning(false)
    }
  }

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Play className="h-5 w-5" />
            Run Workflow
          </CardTitle>
          <CardDescription>
            Execute this sequential workflow with the specified parameters
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {firstStepType === 'agent' && (
            <div className="space-y-2">
              <Label htmlFor="message">Message for Agent</Label>
              <Input
                id="message"
                value={message}
                onChange={(e) => setMessage(e.target.value)}
                placeholder="Enter your message here..."
                disabled={isRunning}
              />
            </div>
          )}

          <Button
            onClick={runWorkflow}
            disabled={isRunning || (firstStepType === 'agent' && !message.trim())}
            className="w-full"
          >
            {isRunning ? (
              <>
                <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                Running Workflow...
              </>
            ) : (
              <>
                <Play className="h-4 w-4 mr-2" />
                Run Workflow
              </>
            )}
          </Button>

          {result && (
            <Card className="mt-4">
              <CardHeader>
                <CardTitle className="text-sm">Result</CardTitle>
              </CardHeader>
              <CardContent>
                <pre className="text-xs bg-muted p-2 rounded overflow-auto max-h-40">
                  {JSON.stringify(result, null, 2)}
                </pre>
              </CardContent>
            </Card>
          )}
        </CardContent>
      </Card>
    </div>
  )
}