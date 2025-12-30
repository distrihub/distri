import AuthorizeButton from '@/components/AuthorizeButton'

export default function AuthPage() {
  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex flex-col items-center justify-center">
      <div className="flex flex-col items-center gap-8">
        <img
          src="/images/blinksheets.png"
          alt="Blink AI"
          className="h-16 w-auto"
        />
        <AuthorizeButton />
      </div>
    </div>
  )
}