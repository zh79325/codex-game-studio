using System;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using GLTFast;
using UnityEngine;

namespace ModelPreview.Runtime.Importing
{
    public sealed class RuntimeGltfLoader : MonoBehaviour
    {
        [SerializeField] private string configuredPath = string.Empty;
        [SerializeField] private string defaultInstanceName = "Runtime Model";

        private CancellationTokenSource _activeLoadCancellation;
        private int _loadGeneration;
        private bool _disposed;

        public event Action<RuntimeModelLoadResult> LoadCompleted;

        public string ConfiguredPath
        {
            get => configuredPath;
            set => configuredPath = value ?? string.Empty;
        }

        public RuntimeModelLoadState State { get; private set; } = RuntimeModelLoadState.Idle;
        public RuntimeModelHandle Current { get; private set; }
        public RuntimeModelLoadResult LastResult { get; private set; }
        public bool IsLoading => State == RuntimeModelLoadState.Loading;

        public Task<RuntimeModelLoadResult> LoadConfiguredPathAsync(CancellationToken cancellationToken = default)
        {
            return LoadAsync(
                new RuntimeModelLoadRequest(configuredPath, transform, defaultInstanceName),
                cancellationToken);
        }

        public Task<RuntimeModelLoadResult> LoadAsync(
            string filePath,
            Transform parent = null,
            bool forceReload = false,
            CancellationToken cancellationToken = default)
        {
            return LoadAsync(
                new RuntimeModelLoadRequest(filePath, parent, null, forceReload),
                cancellationToken);
        }

        public async Task<RuntimeModelLoadResult> LoadAsync(
            RuntimeModelLoadRequest request,
            CancellationToken cancellationToken = default)
        {
            if (_disposed)
            {
                return Publish(RuntimeModelLoadResult.Failure(
                    RuntimeModelLoadErrorCode.LoaderDisposed,
                    "The runtime model loader has been disposed."));
            }

            if (request == null)
            {
                return Publish(RuntimeModelLoadResult.Failure(
                    RuntimeModelLoadErrorCode.InvalidPath,
                    "The model load request is null."));
            }

            if (!TryNormalizePath(request.FilePath, out var normalizedPath, out var validationFailure))
            {
                State = Current == null ? RuntimeModelLoadState.Failed : RuntimeModelLoadState.Loaded;
                return Publish(validationFailure);
            }

            if (cancellationToken.IsCancellationRequested)
            {
                State = Current == null ? RuntimeModelLoadState.Canceled : RuntimeModelLoadState.Loaded;
                return Publish(RuntimeModelLoadResult.Failure(
                    RuntimeModelLoadErrorCode.Canceled,
                    "The model load request was canceled before it started."));
            }

            if (!request.ForceReload
                && Current != null
                && !Current.IsDisposed
                && string.Equals(Current.SourcePath, normalizedPath, StringComparison.OrdinalIgnoreCase))
            {
                if (Current.Root != null && Current.Root.transform.parent != request.Parent)
                {
                    Current.Root.transform.SetParent(request.Parent, false);
                }

                State = RuntimeModelLoadState.Loaded;
                return Publish(RuntimeModelLoadResult.Success(Current, true));
            }

            CancelActiveLoad();
            var generation = ++_loadGeneration;
            var linkedCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            _activeLoadCancellation = linkedCancellation;
            var token = linkedCancellation.Token;

            GltfImport pendingImport = null;
            GameObject pendingRoot = null;
            State = RuntimeModelLoadState.Loading;

            try
            {
                pendingImport = new GltfImport();
                var loaded = await pendingImport.Load(
                    new Uri(normalizedPath, UriKind.Absolute),
                    cancellationToken: token);
                token.ThrowIfCancellationRequested();
                ThrowIfStale(generation, token);

                if (!loaded)
                {
                    var failure = RuntimeModelLoadResult.Failure(
                        RuntimeModelLoadErrorCode.ImportFailed,
                        "glTFast could not parse the model file.");
                    SetTerminalState(generation, RuntimeModelLoadState.Failed);
                    return PublishIfCurrent(generation, failure);
                }

                var instanceName = string.IsNullOrWhiteSpace(request.InstanceName)
                    ? Path.GetFileNameWithoutExtension(normalizedPath)
                    : request.InstanceName.Trim();
                pendingRoot = new GameObject(instanceName);
                pendingRoot.SetActive(false);
                pendingRoot.transform.SetParent(request.Parent, false);

                // The GLB may contain baked legacy clips. They are still retained
                // on RuntimeModelHandle for inspection, but must not instantiate an
                // Animation component: it would auto-play alongside the Humanoid
                // Animator and both systems would write the same bones every frame.
                var instantiationSettings = new InstantiationSettings
                {
                    Mask = ComponentType.All & ~ComponentType.Animation
                };
                var instantiator = new GameObjectInstantiator(
                    pendingImport,
                    pendingRoot.transform,
                    settings: instantiationSettings);
                var instantiated = await pendingImport.InstantiateMainSceneAsync(
                    instantiator,
                    token);
                token.ThrowIfCancellationRequested();
                ThrowIfStale(generation, token);

                if (!instantiated)
                {
                    var failure = RuntimeModelLoadResult.Failure(
                        RuntimeModelLoadErrorCode.InstantiationFailed,
                        "glTFast parsed the file but could not instantiate its main scene.");
                    SetTerminalState(generation, RuntimeModelLoadState.Failed);
                    return PublishIfCurrent(generation, failure);
                }

                pendingRoot.SetActive(true);
                ThrowIfStale(generation, token);
                var nextHandle = new RuntimeModelHandle(
                    normalizedPath,
                    pendingImport,
                    pendingRoot,
                    pendingImport.GetAnimationClips());
                pendingImport = null;
                pendingRoot = null;

                var previousHandle = Current;
                Current = nextHandle;
                previousHandle?.Dispose();
                State = RuntimeModelLoadState.Loaded;

                return PublishIfCurrent(
                    generation,
                    RuntimeModelLoadResult.Success(nextHandle));
            }
            catch (OperationCanceledException exception)
            {
                var canceled = RuntimeModelLoadResult.Failure(
                    RuntimeModelLoadErrorCode.Canceled,
                    "The model load request was canceled.",
                    exception);
                SetTerminalState(generation, RuntimeModelLoadState.Canceled);
                return PublishIfCurrent(generation, canceled);
            }
            catch (Exception exception)
            {
                var failure = RuntimeModelLoadResult.Failure(
                    RuntimeModelLoadErrorCode.Unexpected,
                    exception.Message,
                    exception);
                SetTerminalState(generation, RuntimeModelLoadState.Failed);
                return PublishIfCurrent(generation, failure);
            }
            finally
            {
                DestroyPending(pendingRoot, pendingImport);
                if (ReferenceEquals(_activeLoadCancellation, linkedCancellation))
                {
                    _activeLoadCancellation = null;
                }

                linkedCancellation.Dispose();
            }
        }

public void CancelLoading()
        {
            if (_disposed || _activeLoadCancellation == null)
            {
                return;
            }

            _loadGeneration++;
            CancelActiveLoad();
            State = Current == null ? RuntimeModelLoadState.Canceled : RuntimeModelLoadState.Loaded;
            Publish(RuntimeModelLoadResult.Failure(
                RuntimeModelLoadErrorCode.Canceled,
                "The active model load request was canceled."));
        }

public void Unload()
        {
            if (_disposed)
            {
                return;
            }

            _loadGeneration++;
            CancelActiveLoad();
            Current?.Dispose();
            Current = null;
            LastResult = default;
            State = RuntimeModelLoadState.Idle;
        }

        private static bool TryNormalizePath(
            string rawPath,
            out string normalizedPath,
            out RuntimeModelLoadResult failure)
        {
            normalizedPath = string.Empty;
            failure = default;

            if (string.IsNullOrWhiteSpace(rawPath))
            {
                failure = RuntimeModelLoadResult.Failure(
                    RuntimeModelLoadErrorCode.InvalidPath,
                    "The model file path is empty.");
                return false;
            }

            try
            {
                var candidate = rawPath.Trim().Trim('"');
                if (Uri.TryCreate(candidate, UriKind.Absolute, out var inputUri) && inputUri.IsFile)
                {
                    candidate = inputUri.LocalPath;
                }

                if (!Path.IsPathRooted(candidate))
                {
                    failure = RuntimeModelLoadResult.Failure(
                        RuntimeModelLoadErrorCode.InvalidPath,
                        "The model file path must be absolute.");
                    return false;
                }

                normalizedPath = Path.GetFullPath(candidate);
                var extension = Path.GetExtension(normalizedPath);
                if (!extension.Equals(".glb", StringComparison.OrdinalIgnoreCase)
                    && !extension.Equals(".gltf", StringComparison.OrdinalIgnoreCase))
                {
                    failure = RuntimeModelLoadResult.Failure(
                        RuntimeModelLoadErrorCode.UnsupportedFormat,
                        "Only GLB and GLTF files are supported.");
                    return false;
                }

                if (!File.Exists(normalizedPath))
                {
                    failure = RuntimeModelLoadResult.Failure(
                        RuntimeModelLoadErrorCode.FileNotFound,
                        "The model file does not exist: " + normalizedPath);
                    return false;
                }

                return true;
            }
            catch (Exception exception) when (
                exception is ArgumentException
                || exception is NotSupportedException
                || exception is PathTooLongException)
            {
                failure = RuntimeModelLoadResult.Failure(
                    RuntimeModelLoadErrorCode.InvalidPath,
                    exception.Message,
                    exception);
                return false;
            }
        }

        private void ThrowIfStale(int generation, CancellationToken token)
        {
            if (_disposed || generation != _loadGeneration)
            {
                throw new OperationCanceledException(token);
            }
        }

        private void SetTerminalState(int generation, RuntimeModelLoadState emptyState)
        {
            if (generation == _loadGeneration && !_disposed)
            {
                State = Current == null ? emptyState : RuntimeModelLoadState.Loaded;
            }
        }

        private RuntimeModelLoadResult PublishIfCurrent(int generation, RuntimeModelLoadResult result)
        {
            return generation == _loadGeneration && !_disposed ? Publish(result) : result;
        }

        private RuntimeModelLoadResult Publish(RuntimeModelLoadResult result)
        {
            LastResult = result;
            try
            {
                LoadCompleted?.Invoke(result);
            }
            catch (Exception exception)
            {
                Debug.LogException(exception, this);
            }

            return result;
        }

        private void CancelActiveLoad()
        {
            _activeLoadCancellation?.Cancel();
        }

        private static void DestroyPending(GameObject root, GltfImport import)
        {
            if (root != null)
            {
                root.SetActive(false);
                if (Application.isPlaying)
                {
                    Destroy(root);
                }
                else
                {
                    DestroyImmediate(root);
                }
            }

            import?.Dispose();
        }

        private void OnDestroy()
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            _loadGeneration++;
            CancelActiveLoad();
            Current?.Dispose();
            Current = null;
            State = RuntimeModelLoadState.Disposed;
        }
    }
}
