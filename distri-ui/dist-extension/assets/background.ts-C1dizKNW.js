chrome.runtime.onInstalled.addListener(()=>{console.log("Distri Extension installed")});chrome.action.onClicked.addListener(e=>{console.log("Extension clicked on tab:",e.id)});
