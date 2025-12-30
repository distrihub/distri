import { BACKEND_URL } from "@/constants";
import { GooglePickerConfig, PickerDocument } from "./GoogleSheetPicker";

export const getDriveToken = async (pickerConfig: GooglePickerConfig | null): Promise<string | null> => {
  if (!pickerConfig) {
    return null;
  }

  const LOCALSTORAGE_KEY = `google_drive_token_${pickerConfig.client_id}`;
  const SCOPES = 'https://www.googleapis.com/auth/drive.file';

  // Check localStorage for a valid token
  const stored = localStorage.getItem(LOCALSTORAGE_KEY);
  if (stored) {
    try {
      const { access_token, expires_at } = JSON.parse(stored);
      if (access_token && expires_at && Date.now() < expires_at) {
        return access_token;
      }
    } catch (e) {
      // Ignore parse errors, fallback to requesting new token
    }
  }

  // No valid token, request a new one
  return new Promise((resolve, reject) => {
    const tokenClient = window.google.accounts.oauth2.initTokenClient({
      client_id: pickerConfig.client_id,
      scope: SCOPES,
      callback: (tokenResponse: any) => {
        console.log("tokenResponse", tokenResponse)
        if (tokenResponse.error) {
          reject(tokenResponse.error);
        } else {
          const accessToken = tokenResponse.access_token;
          // Google returns expires_in in seconds
          const expiresIn = tokenResponse.expires_in || 3600; // fallback 1 hour
          const expiresAt = Date.now() + expiresIn * 1000 - 60000; // 1 min early
          // Store in localStorage
          localStorage.setItem(LOCALSTORAGE_KEY, JSON.stringify({
            access_token: accessToken,
            expires_at: expiresAt
          }));
          resolve(accessToken);
        }
      }
    });
    tokenClient.requestAccessToken({ prompt: '' });
  });
}

export const getFileFromGoogle = async (fileId: string, accessToken: string | null): Promise<PickerDocument | null> => {
  if (!accessToken) {
    console.log("No drive token, returning null")
    return null;
  }
  try {
    const response = await fetch(`https://www.googleapis.com/drive/v3/files/${fileId}?fields=id,name,mimeType,iconLink,webViewLink`, {
      headers: {
        'Authorization': `Bearer ${accessToken}`
      }
    });
    if (!response.ok) {
      return null
    }

    return await response.json()
  } catch (err) {
    console.error('Error fetching file:', err)
    return null
  }
};
export const getFile = async (fileId: string, token: string | null): Promise<PickerDocument | null> => {
  if (!token) {
    console.log("No token, returning null")
    return null
  }

  try {
    const response = await fetch(`${BACKEND_URL}/api/files/${fileId}`, {
      method: 'GET',
      headers: {
        'Authorization': `Bearer ${token}`
      }
    })

    if (!response.ok) {
      return null
    }

    return await response.json()
  } catch (err) {
    console.error('Error fetching files:', err)
    return null
  }
};


export const getFiles = async (token: string | null): Promise<PickerDocument[]> => {
  if (!token) return []

  try {
    const response = await fetch(`${BACKEND_URL}/api/files`, {
      method: 'GET',
      headers: {
        'Authorization': `Bearer ${token}`
      }
    })

    if (!response.ok) {
      return []
    }

    return await response.json()
  } catch (err) {
    console.error('Error fetching files:', err)
    return []
  }
}

export const getAccessToken = async (token: string | null): Promise<string | null> => {
  if (!token) return null

  try {
    const response = await fetch(`${BACKEND_URL}/api/access_token`, {
      method: 'GET',
      headers: {
        'Authorization': `Bearer ${token}`
      }
    })

    if (!response.ok) {
      return null
    }

    return await response.json()
  } catch (err) {
    console.error('Error fetching access token:', err)
    return null
  }
}