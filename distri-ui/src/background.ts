chrome.runtime.onInstalled.addListener(() => {
  console.log('Distri Extension installed')
})

chrome.action.onClicked.addListener((tab) => {
  console.log('Extension clicked on tab:', tab.id)
})