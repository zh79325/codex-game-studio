using System;
using UnityEngine;

namespace ModelPreview.Runtime.Importing
{
    public enum RuntimeModelLoadState
    {
        Idle,
        Loading,
        Loaded,
        Failed,
        Canceled,
        Disposed
    }

    public enum RuntimeModelLoadErrorCode
    {
        None,
        InvalidPath,
        FileNotFound,
        UnsupportedFormat,
        ImportFailed,
        InstantiationFailed,
        Canceled,
        LoaderDisposed,
        Unexpected
    }

    public sealed class RuntimeModelLoadRequest
    {
        public RuntimeModelLoadRequest(
            string filePath,
            Transform parent = null,
            string instanceName = null,
            bool forceReload = false)
        {
            FilePath = filePath;
            Parent = parent;
            InstanceName = instanceName;
            ForceReload = forceReload;
        }

        public string FilePath { get; }
        public Transform Parent { get; }
        public string InstanceName { get; }
        public bool ForceReload { get; }
    }

    public readonly struct RuntimeModelLoadResult
    {
        private RuntimeModelLoadResult(
            RuntimeModelHandle handle,
            RuntimeModelLoadErrorCode errorCode,
            string errorMessage,
            Exception exception,
            bool reused)
        {
            Handle = handle;
            ErrorCode = errorCode;
            ErrorMessage = errorMessage ?? string.Empty;
            Exception = exception;
            Reused = reused;
        }

        public bool Succeeded => ErrorCode == RuntimeModelLoadErrorCode.None && Handle != null;
        public bool Reused { get; }
        public RuntimeModelHandle Handle { get; }
        public RuntimeModelLoadErrorCode ErrorCode { get; }
        public string ErrorMessage { get; }
        public Exception Exception { get; }

        public static RuntimeModelLoadResult Success(RuntimeModelHandle handle, bool reused = false)
        {
            if (handle == null)
            {
                throw new ArgumentNullException(nameof(handle));
            }

            return new RuntimeModelLoadResult(
                handle,
                RuntimeModelLoadErrorCode.None,
                string.Empty,
                null,
                reused);
        }

        public static RuntimeModelLoadResult Failure(
            RuntimeModelLoadErrorCode errorCode,
            string message,
            Exception exception = null)
        {
            return new RuntimeModelLoadResult(null, errorCode, message, exception, false);
        }
    }
}
