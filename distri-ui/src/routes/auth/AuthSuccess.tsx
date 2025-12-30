import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";

export default function AuthSuccess() {
  const [isEmbedded, setIsEmbedded] = useState(false);
  const navigate = useNavigate();

  useEffect(() => {
    // Check if page is embedded in an iframe
    if (window !== window.parent) {
      setIsEmbedded(true);
    } else {
      // Redirect to home page if not embedded
      navigate('/');
    }
  }, [navigate]);

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex flex-col items-center justify-center">
      <div className="text-center p-8 rounded-lg">
        <div className="mb-8">
          <img
            src="/images/blinksheets.png"
            alt="Logo"
            width={120}
            height={120}
            className="mx-auto"
          />
        </div>
        <h1 className="text-2xl font-semibold text-gray-800 dark:text-gray-100 mb-4">
          Authentication Successful!
        </h1>
        <p className="text-gray-600 dark:text-gray-300 mb-2">
          {isEmbedded ?
            "You can now close this window." :
            "Redirecting you to the home page..."}
        </p>
      </div>
    </div>
  );
}