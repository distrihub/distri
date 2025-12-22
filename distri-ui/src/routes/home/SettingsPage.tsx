import { ConfigurationPanel } from '@distri/react'

const SettingsPage = () => {
  return (
    <div className="flex-1 overflow-auto">
      <div className="mx-auto w-full max-w-4xl px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
        <ConfigurationPanel title="Settings" />
      </div>
    </div>
  )
}

export default SettingsPage


