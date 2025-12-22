const STORAGE_PREFIX = 'distri:skill-designer-seed:'

const isBrowser = () => typeof window !== 'undefined' && typeof window.sessionStorage !== 'undefined'

export const persistDesignerSeed = (skillId: string, prompt: string) => {
  if (!skillId || !prompt.trim() || !isBrowser()) {
    return
  }
  try {
    window.sessionStorage.setItem(`${STORAGE_PREFIX}${skillId}`, prompt)
  } catch (error) {
    console.warn('Failed to persist designer seed', error)
  }
}

export const consumeDesignerSeed = (skillId: string): string | null => {
  if (!skillId || !isBrowser()) {
    return null
  }

  const key = `${STORAGE_PREFIX}${skillId}`
  try {
    const value = window.sessionStorage.getItem(key)
    if (value) {
      window.sessionStorage.removeItem(key)
      return value
    }
  } catch (error) {
    console.warn('Failed to consume designer seed', error)
  }
  return null
}
