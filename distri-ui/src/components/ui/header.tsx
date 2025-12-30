import React from 'react'
import { SidebarTrigger } from './sidebar'

interface HeaderProps {
  title?: string
  subtitle?: React.ReactNode
  leftElement?: React.ReactNode
  rightElement?: React.ReactNode
  className?: string
}

export function Header({
  title,
  subtitle,
  leftElement,
  rightElement,
  className = ""
}: HeaderProps) {
  return (
    <header className={`flex flex-col shrink-0 px-4 py-3 ${className}`}>
      <div className="flex items-center gap-2">
        <SidebarTrigger className="-ml-1" />

        <div className="flex items-center gap-2 flex-1">
          {leftElement && (
            <div className="flex items-center gap-2">
              {leftElement}
            </div>
          )}
          {title && (
            <h1 className="text-lg font-semibold">{title}</h1>
          )}
        </div>

        {rightElement && (
          <div className="flex items-center gap-2">
            {rightElement}
          </div>
        )}
      </div>

      {subtitle && (
        <div className="flex items-center gap-2 mt-1">
          {subtitle}
        </div>
      )}
    </header>
  )
} 