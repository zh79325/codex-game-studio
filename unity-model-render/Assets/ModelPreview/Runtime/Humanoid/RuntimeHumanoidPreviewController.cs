using System;
using System.Threading;
using System.Threading.Tasks;
using ModelPreview.Runtime.Importing;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.Universal;

using UnityEngine.EventSystems;
using UnityEngine.UI;

namespace ModelPreview.Runtime.Humanoid
{
    [DisallowMultipleComponent]
    [RequireComponent(typeof(RuntimeGltfLoader))]
    [RequireComponent(typeof(RuntimeHumanoidAnimationPlayer))]
    public sealed class RuntimeHumanoidPreviewController : MonoBehaviour, IBeginDragHandler, IDragHandler, IEndDragHandler
    {
        [Header("Model")]
        [SerializeField] private string modelFilePath = string.Empty;
        [SerializeField] private HumanoidAvatarProfile avatarProfile;
        [SerializeField] private bool loadOnStart = true;

        [Header("Preview")]
        [SerializeField] private Camera previewCamera;
        [SerializeField] private Vector3 cameraDirection = Vector3.back;
        [SerializeField, Min(1.5f)] private float cameraDistanceMultiplier = 3.2f;
        [SerializeField] private bool createGround = true;
        [SerializeField] private bool createRuntimeUi = true;
        [SerializeField] private bool configureGameLighting = true;
        [SerializeField] private bool enableHighQualityCamera = true;

        [Header("Orbit Control")]
        [SerializeField] private bool enableOrbitControl = true;
        [SerializeField, Min(0.01f)] private float orbitDegreesPerPixel = 0.25f;


        private RuntimeGltfLoader _modelLoader;
        private RuntimeHumanoidAnimationPlayer _animationPlayer;
        private CancellationTokenSource _lifetimeCancellation;
        private Text _statusText;
        private Button _idleButton;
        private Button _walkButton;
        private Button _runButton;
        private Material _groundMaterial;
        private Material _originalSkyboxMaterial;
        private Material _runtimeSkyboxMaterial;
        private GameObject _orbitDragSurface;
        private Vector3 _orbitPivot;
        private Vector3 _orbitOffset;
        private bool _orbitReady;
        private bool _isOrbitDragging;



        public RuntimeModelHandle CurrentModel => _modelLoader != null ? _modelLoader.Current : null;
        public RuntimeHumanoidAnimationPlayer AnimationPlayer => _animationPlayer;

        private void Awake()
        {
            _modelLoader = GetComponent<RuntimeGltfLoader>();
            _animationPlayer = GetComponent<RuntimeHumanoidAnimationPlayer>();
            _animationPlayer.AnimationChanged += OnAnimationChanged;

            if (createRuntimeUi)
            {
                BuildRuntimeUi();
            }
        }

        private async void Start()
        {
            if (loadOnStart)
            {
                await LoadConfiguredModelAsync();
            }
        }

        public async Task<bool> LoadConfiguredModelAsync()
        {
            _lifetimeCancellation?.Cancel();
            _lifetimeCancellation?.Dispose();
            _lifetimeCancellation = new CancellationTokenSource();

            var path = string.IsNullOrWhiteSpace(modelFilePath)
                ? _modelLoader.ConfiguredPath
                : modelFilePath.Trim();
            SetStatus("Loading model...");
            SetButtonsInteractable(false);

            RuntimeModelLoadResult loadResult;
            try
            {
                loadResult = await _modelLoader.LoadAsync(
                    path,
                    transform,
                    false,
                    _lifetimeCancellation.Token);
            }
            catch (OperationCanceledException)
            {
                SetStatus("Loading canceled.");
                return false;
            }

            if (!loadResult.Succeeded || loadResult.Handle == null)
            {
                SetStatus("Load failed: " + loadResult.ErrorMessage);
                return false;
            }

            var avatarResult = RuntimeHumanoidAvatarBuilder.Build(loadResult.Handle, avatarProfile);
            if (!avatarResult.Succeeded)
            {
                SetStatus("Avatar failed: " + avatarResult.Message);
                return false;
            }

            if (!_animationPlayer.Configure(avatarResult.Animator, loadResult.Handle.AnimationClips))
            {
                SetStatus("No supported Idle, Walk, or Run animation was found.");
                return false;
            }

            ConfigureOriginalArtMaterials(loadResult.Handle.Root);
            
var camera = ConfigureCamera(loadResult.Handle.Root);
            if (configureGameLighting && camera != null)
            {
                ConfigureGameLighting(loadResult.Handle.Root, camera);
            }
            if (createGround)
            {
                CreatePreviewGround(loadResult.Handle.Root);
            }

            RefreshButtons();
            OnAnimationChanged(_animationPlayer.CurrentRole, null);
            return true;
        }

        public void PlayIdle()
        {
            Play(HumanoidAnimationRole.Idle);
        }

        public void PlayWalk()
        {
            Play(HumanoidAnimationRole.Walk);
        }

        public void PlayRun()
        {
            Play(HumanoidAnimationRole.Run);
        }

public void RotateView(float deltaYawDegrees)
        {
            if (!enableOrbitControl || !_orbitReady || Mathf.Approximately(deltaYawDegrees, 0f))
            {
                return;
            }

            _orbitOffset = Quaternion.AngleAxis(deltaYawDegrees, Vector3.up) * _orbitOffset;
            ApplyOrbitCamera();
        }


        private void Play(HumanoidAnimationRole role)
        {
            if (!_animationPlayer.Play(role))
            {
                SetStatus(role + " animation is unavailable.");
            }
        }

        private void OnAnimationChanged(HumanoidAnimationRole role, AnimationClip clip)
        {
            var clipName = clip != null ? clip.name : _animationPlayer.CurrentClipName;
            SetStatus("Playing: " + role + (string.IsNullOrEmpty(clipName) ? string.Empty : "  (" + clipName + ")"));
            HighlightActiveButton(role);
        }

public void OnBeginDrag(PointerEventData eventData)
        {
            _isOrbitDragging = enableOrbitControl
                && _orbitReady
                && eventData != null
                && eventData.button == PointerEventData.InputButton.Left
                && _orbitDragSurface != null
                && eventData.pointerPressRaycast.gameObject == _orbitDragSurface;
        }

public void OnDrag(PointerEventData eventData)
        {
            if (!_isOrbitDragging || eventData == null)
            {
                return;
            }

            RotateView(eventData.delta.x * orbitDegreesPerPixel);
        }

public void OnEndDrag(PointerEventData eventData)
        {
            _isOrbitDragging = false;
        }




        private void BuildRuntimeUi()
        {
            if (transform.Find("Animation Preview UI") != null)
            {
                return;
            }

            EnsureEventSystem();

            var canvasObject = new GameObject(
                "Animation Preview UI",
                typeof(RectTransform),
                typeof(Canvas),
                typeof(CanvasScaler),
                typeof(GraphicRaycaster));
            canvasObject.transform.SetParent(transform, false);
            var canvas = canvasObject.GetComponent<Canvas>();
            canvas.renderMode = RenderMode.ScreenSpaceOverlay;
            canvas.sortingOrder = 100;
            var scaler = canvasObject.GetComponent<CanvasScaler>();
            scaler.uiScaleMode = CanvasScaler.ScaleMode.ScaleWithScreenSize;
            scaler.referenceResolution = new Vector2(1280f, 720f);
            scaler.screenMatchMode = CanvasScaler.ScreenMatchMode.MatchWidthOrHeight;
            scaler.matchWidthOrHeight = 0.5f;

            var orbitSurfaceObject = new GameObject(
                "Orbit Drag Surface",
                typeof(RectTransform),
                typeof(Image));
            orbitSurfaceObject.transform.SetParent(canvasObject.transform, false);
            orbitSurfaceObject.transform.SetAsFirstSibling();
            var orbitSurfaceRect = orbitSurfaceObject.GetComponent<RectTransform>();
            orbitSurfaceRect.anchorMin = Vector2.zero;
            orbitSurfaceRect.anchorMax = Vector2.one;
            orbitSurfaceRect.offsetMin = Vector2.zero;
            orbitSurfaceRect.offsetMax = Vector2.zero;
            var orbitSurfaceImage = orbitSurfaceObject.GetComponent<Image>();
            orbitSurfaceImage.color = Color.clear;
            orbitSurfaceImage.raycastTarget = true;
            _orbitDragSurface = orbitSurfaceObject;

            var statusObject = new GameObject("Status", typeof(RectTransform), typeof(Text), typeof(Outline));
            statusObject.transform.SetParent(canvasObject.transform, false);
            var statusRect = statusObject.GetComponent<RectTransform>();
            statusRect.anchorMin = new Vector2(0.5f, 0f);
            statusRect.anchorMax = new Vector2(0.5f, 0f);
            statusRect.pivot = new Vector2(0.5f, 0f);
            statusRect.sizeDelta = new Vector2(720f, 42f);
            statusRect.anchoredPosition = new Vector2(0f, 112f);
            _statusText = statusObject.GetComponent<Text>();
            _statusText.font = Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _statusText.fontSize = 21;
            _statusText.alignment = TextAnchor.MiddleCenter;
            _statusText.color = Color.white;
            _statusText.raycastTarget = false;
            statusObject.GetComponent<Outline>().effectColor = new Color(0f, 0f, 0f, 0.75f);

            var panelObject = new GameObject(
                "Animation Buttons",
                typeof(RectTransform),
                typeof(Image),
                typeof(HorizontalLayoutGroup));
            panelObject.transform.SetParent(canvasObject.transform, false);
            var panelRect = panelObject.GetComponent<RectTransform>();
            panelRect.anchorMin = new Vector2(0.5f, 0f);
            panelRect.anchorMax = new Vector2(0.5f, 0f);
            panelRect.pivot = new Vector2(0.5f, 0f);
            panelRect.sizeDelta = new Vector2(560f, 82f);
            panelRect.anchoredPosition = new Vector2(0f, 24f);
            var panelImage = panelObject.GetComponent<Image>();
            panelImage.color = new Color(0.035f, 0.045f, 0.065f, 0.9f);
            var layout = panelObject.GetComponent<HorizontalLayoutGroup>();
            layout.padding = new RectOffset(16, 16, 14, 14);
            layout.spacing = 14f;
            layout.childAlignment = TextAnchor.MiddleCenter;
            layout.childControlWidth = true;
            layout.childControlHeight = true;
            layout.childForceExpandWidth = true;
            layout.childForceExpandHeight = true;

            _idleButton = CreateButton(panelObject.transform, "Idle", HumanoidAnimationRole.Idle);
            _walkButton = CreateButton(panelObject.transform, "Walk", HumanoidAnimationRole.Walk);
            _runButton = CreateButton(panelObject.transform, "Run", HumanoidAnimationRole.Run);
            SetStatus("Waiting to load model...");
            SetButtonsInteractable(false);
        }

        private Button CreateButton(
            Transform parent,
            string label,
            HumanoidAnimationRole role)
        {
            var buttonObject = new GameObject(
                label + " Button",
                typeof(RectTransform),
                typeof(Image),
                typeof(Button),
                typeof(LayoutElement));
            buttonObject.transform.SetParent(parent, false);
            var image = buttonObject.GetComponent<Image>();
            image.color = new Color(0.18f, 0.22f, 0.3f, 1f);
            var button = buttonObject.GetComponent<Button>();
            button.targetGraphic = image;
            var colors = button.colors;
            colors.normalColor = new Color(0.18f, 0.22f, 0.3f, 1f);
            colors.highlightedColor = new Color(0.28f, 0.4f, 0.58f, 1f);
            colors.pressedColor = new Color(0.12f, 0.55f, 0.78f, 1f);
            colors.disabledColor = new Color(0.1f, 0.11f, 0.14f, 0.55f);
            button.colors = colors;
            button.onClick.AddListener(() => Play(role));

            var textObject = new GameObject("Label", typeof(RectTransform), typeof(Text));
            textObject.transform.SetParent(buttonObject.transform, false);
            var textRect = textObject.GetComponent<RectTransform>();
            textRect.anchorMin = Vector2.zero;
            textRect.anchorMax = Vector2.one;
            textRect.offsetMin = Vector2.zero;
            textRect.offsetMax = Vector2.zero;
            var text = textObject.GetComponent<Text>();
            text.font = Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            text.fontSize = 24;
            text.alignment = TextAnchor.MiddleCenter;
            text.color = Color.white;
            text.text = label;
            text.raycastTarget = false;
            return button;
        }

        private void RefreshButtons()
        {
            if (_idleButton != null)
            {
                _idleButton.interactable = _animationPlayer.IsAvailable(HumanoidAnimationRole.Idle);
            }
            if (_walkButton != null)
            {
                _walkButton.interactable = _animationPlayer.IsAvailable(HumanoidAnimationRole.Walk);
            }
            if (_runButton != null)
            {
                _runButton.interactable = _animationPlayer.IsAvailable(HumanoidAnimationRole.Run);
            }
        }

        private void SetButtonsInteractable(bool value)
        {
            if (_idleButton != null)
            {
                _idleButton.interactable = value;
            }
            if (_walkButton != null)
            {
                _walkButton.interactable = value;
            }
            if (_runButton != null)
            {
                _runButton.interactable = value;
            }
        }

        private void HighlightActiveButton(HumanoidAnimationRole activeRole)
        {
            SetButtonHighlight(_idleButton, activeRole == HumanoidAnimationRole.Idle);
            SetButtonHighlight(_walkButton, activeRole == HumanoidAnimationRole.Walk);
            SetButtonHighlight(_runButton, activeRole == HumanoidAnimationRole.Run);
        }

        private static void SetButtonHighlight(Button button, bool active)
        {
            if (button == null)
            {
                return;
            }

            var colors = button.colors;
            colors.normalColor = active
                ? new Color(0.08f, 0.56f, 0.82f, 1f)
                : new Color(0.18f, 0.22f, 0.3f, 1f);
            button.colors = colors;
            if (button.targetGraphic != null)
            {
                button.targetGraphic.color = colors.normalColor;
            }
        }

        private void SetStatus(string message)
        {
            if (_statusText != null)
            {
                _statusText.text = message;
            }
        }

private void ApplyOrbitCamera()
        {
            var camera = previewCamera != null ? previewCamera : Camera.main;
            if (camera == null || !_orbitReady || _orbitOffset.sqrMagnitude < 0.001f)
            {
                return;
            }

            var position = _orbitPivot + _orbitOffset;
            var rotation = Quaternion.LookRotation(_orbitPivot - position, Vector3.up);
            camera.transform.SetPositionAndRotation(position, rotation);
        }


private static void ConfigureOriginalArtMaterials(GameObject modelRoot)
        {
            if (modelRoot == null)
            {
                return;
            }

            var configuredMaterials = new System.Collections.Generic.HashSet<Material>();
            foreach (var renderer in modelRoot.GetComponentsInChildren<Renderer>(true))
            {
                foreach (var material in renderer.sharedMaterials)
                {
                    if (material == null || !configuredMaterials.Add(material))
                    {
                        continue;
                    }

                    if (material.HasProperty("metallicFactor"))
                    {
                        material.SetFloat("metallicFactor", 0f);
                    }
                    if (material.HasProperty("roughnessFactor"))
                    {
                        material.SetFloat("roughnessFactor", 0.9f);
                    }
                    if (material.HasProperty("metallicRoughnessTexture"))
                    {
                        material.SetTexture("metallicRoughnessTexture", null);
                    }
                    if (material.HasProperty("_Metallic"))
                    {
                        material.SetFloat("_Metallic", 0f);
                    }
                    if (material.HasProperty("_Smoothness"))
                    {
                        material.SetFloat("_Smoothness", 0.1f);
                    }
                }
            }
        }

        
private Camera ConfigureCamera(GameObject modelRoot)
        {
            var camera = previewCamera != null ? previewCamera : Camera.main;
            if (camera == null)
            {
                var cameraObject = new GameObject("Runtime Preview Camera", typeof(Camera), typeof(AudioListener));
                camera = cameraObject.GetComponent<Camera>();
                cameraObject.tag = "MainCamera";
            }

            if (!TryCalculateBounds(modelRoot, out var bounds))
            {
                return camera;
            }

            var direction = cameraDirection.sqrMagnitude > 0.001f
                ? cameraDirection.normalized
                : Vector3.back;
            var lookAt = bounds.center - Vector3.up * bounds.extents.y * 0.08f;
            var rotation = Quaternion.LookRotation(-direction, Vector3.up);
            var forward = rotation * Vector3.forward;
            var right = rotation * Vector3.right;
            var up = rotation * Vector3.up;
            var halfWidth = Mathf.Abs(right.x) * bounds.extents.x
                + Mathf.Abs(right.y) * bounds.extents.y
                + Mathf.Abs(right.z) * bounds.extents.z;
            var halfHeight = Mathf.Abs(up.x) * bounds.extents.x
                + Mathf.Abs(up.y) * bounds.extents.y
                + Mathf.Abs(up.z) * bounds.extents.z;
            var halfDepth = Mathf.Abs(forward.x) * bounds.extents.x
                + Mathf.Abs(forward.y) * bounds.extents.y
                + Mathf.Abs(forward.z) * bounds.extents.z;

            camera.fieldOfView = 38f;
            var verticalHalfFov = camera.fieldOfView * 0.5f * Mathf.Deg2Rad;
            var horizontalHalfFov = Mathf.Atan(Mathf.Tan(verticalHalfFov) * Mathf.Max(0.5f, camera.aspect));
            var fitHeight = halfHeight / Mathf.Max(0.01f, Mathf.Tan(verticalHalfFov));
            var fitWidth = halfWidth / Mathf.Max(0.01f, Mathf.Tan(horizontalHalfFov));
            var fittedDistance = (Mathf.Max(fitHeight, fitWidth) + halfDepth) * 1.28f;
            var legacyDistance = Mathf.Max(bounds.extents.x, bounds.extents.y, bounds.extents.z)
                * cameraDistanceMultiplier;
            var distance = Mathf.Max(0.5f, Mathf.Max(legacyDistance, fittedDistance));

            previewCamera = camera;
            _orbitPivot = lookAt;
            _orbitOffset = direction * distance;
            _orbitReady = true;
            ApplyOrbitCamera();
            camera.nearClipPlane = 0.03f;
            camera.farClipPlane = Mathf.Max(30f, distance + halfDepth * 8f);
            camera.clearFlags = CameraClearFlags.Skybox;
            camera.allowHDR = true;
            camera.allowMSAA = true;

            if (enableHighQualityCamera)
            {
                ConfigureCameraRendering(camera);
            }
            return camera;
        }

private static void ConfigureCameraRendering(Camera camera)
        {
            var cameraData = camera.GetUniversalAdditionalCameraData();
            cameraData.renderPostProcessing = true;
            cameraData.antialiasing = AntialiasingMode.SubpixelMorphologicalAntiAliasing;
            cameraData.antialiasingQuality = AntialiasingQuality.High;
            cameraData.stopNaN = true;
            cameraData.dithering = true;
            cameraData.renderShadows = true;
            cameraData.requiresDepthTexture = true;
        }

        private void ConfigureGameLighting(GameObject modelRoot, Camera camera)
        {
            if (!TryCalculateBounds(modelRoot, out var bounds))
            {
                return;
            }

            var lightingRoot = transform.Find("Runtime Preview Lighting");
            if (lightingRoot == null)
            {
                var lightingObject = new GameObject("Runtime Preview Lighting");
                lightingObject.transform.SetParent(transform, false);
                lightingRoot = lightingObject.transform;
            }

            var key = GetOrCreateDirectionalLight(lightingRoot, "Key Light");
            var fill = GetOrCreateDirectionalLight(lightingRoot, "Fill Light");
            var rim = GetOrCreateDirectionalLight(lightingRoot, "Rim Light");

            var toCamera = (camera.transform.position - bounds.center).normalized;
            var cameraRight = camera.transform.right;
            var worldUp = Vector3.up;
            var keySource = (toCamera - cameraRight * 0.55f + worldUp * 0.85f).normalized;
            var fillSource = (toCamera + cameraRight * 0.85f + worldUp * 0.35f).normalized;
            var rimSource = (-toCamera + cameraRight * 0.35f + worldUp * 0.55f).normalized;

            ConfigureDirectionalLight(
                key,
                keySource,
                new Color(1f, 0.97f, 0.93f, 1f),
                0.95f,
                LightShadows.Soft,
                0.85f);
            ConfigureDirectionalLight(
                fill,
                fillSource,
                new Color(0.82f, 0.88f, 1f, 1f),
                0.45f,
                LightShadows.None,
                0f);
            ConfigureDirectionalLight(
                rim,
                rimSource,
                new Color(0.82f, 0.88f, 1f, 1f),
                0.25f,
                LightShadows.None,
                0f);

            foreach (var sceneLight in FindObjectsByType<Light>(FindObjectsSortMode.None))
            {
                if (sceneLight.type == LightType.Directional
                    && sceneLight != key
                    && sceneLight != fill
                    && sceneLight != rim)
                {
                    sceneLight.enabled = false;
                }
            }

            ConfigureRuntimeSkybox();
            
RenderSettings.sun = key;
            RenderSettings.ambientMode = AmbientMode.Trilight;
            RenderSettings.ambientSkyColor = new Color(0.46f, 0.48f, 0.52f, 1f);
            RenderSettings.ambientEquatorColor = new Color(0.30f, 0.31f, 0.34f, 1f);
            RenderSettings.ambientGroundColor = new Color(0.15f, 0.16f, 0.18f, 1f);
            RenderSettings.ambientIntensity = 0.9f;
            RenderSettings.reflectionIntensity = 0.35f;
            RenderSettings.defaultReflectionResolution = 256;
            RenderSettings.fog = false;
            DynamicGI.UpdateEnvironment();
        }

        private static Light GetOrCreateDirectionalLight(Transform parent, string lightName)
        {
            var child = parent.Find(lightName);
            if (child == null)
            {
                var lightObject = new GameObject(lightName, typeof(Light));
                lightObject.transform.SetParent(parent, false);
                child = lightObject.transform;
            }

            var light = child.GetComponent<Light>();
            if (light == null)
            {
                light = child.gameObject.AddComponent<Light>();
            }
            light.type = LightType.Directional;
            light.renderMode = LightRenderMode.ForcePixel;
            light.enabled = true;
            return light;
        }

        private static void ConfigureDirectionalLight(
            Light light,
            Vector3 sourceDirection,
            Color color,
            float intensity,
            LightShadows shadows,
            float shadowStrength)
        {
            light.transform.rotation = Quaternion.LookRotation(-sourceDirection, Vector3.up);
            light.color = color;
            light.intensity = intensity;
            light.shadows = shadows;
            light.shadowStrength = shadowStrength;
            light.shadowBias = 0.03f;
            light.shadowNormalBias = 0.2f;
            light.shadowNearPlane = 0.1f;
        }

        
private void ConfigureRuntimeSkybox()
        {
            if (_runtimeSkyboxMaterial == null)
            {
                _originalSkyboxMaterial = RenderSettings.skybox;
                if (_originalSkyboxMaterial == null)
                {
                    return;
                }

                _runtimeSkyboxMaterial = new Material(_originalSkyboxMaterial)
                {
                    name = "Runtime Preview Skybox"
                };
            }

            if (_runtimeSkyboxMaterial.HasProperty("_SkyTint"))
            {
                _runtimeSkyboxMaterial.SetColor("_SkyTint", new Color(0.48f, 0.50f, 0.55f, 1f));
            }
            if (_runtimeSkyboxMaterial.HasProperty("_GroundColor"))
            {
                _runtimeSkyboxMaterial.SetColor("_GroundColor", new Color(0.22f, 0.23f, 0.25f, 1f));
            }
            if (_runtimeSkyboxMaterial.HasProperty("_Exposure"))
            {
                _runtimeSkyboxMaterial.SetFloat("_Exposure", 0.8f);
            }
            if (_runtimeSkyboxMaterial.HasProperty("_AtmosphereThickness"))
            {
                _runtimeSkyboxMaterial.SetFloat("_AtmosphereThickness", 0.7f);
            }
            RenderSettings.skybox = _runtimeSkyboxMaterial;
        }

        
private void CreatePreviewGround(GameObject modelRoot)
        {
            var existing = transform.Find("Runtime Preview Ground");
            if (existing != null)
            {
                Destroy(existing.gameObject);
            }
            if (_groundMaterial != null)
            {
                Destroy(_groundMaterial);
                _groundMaterial = null;
            }

            if (!TryCalculateBounds(modelRoot, out var bounds))
            {
                return;
            }

            var ground = GameObject.CreatePrimitive(PrimitiveType.Plane);
            ground.name = "Runtime Preview Ground";
            ground.transform.SetParent(transform, true);
            ground.transform.position = new Vector3(bounds.center.x, bounds.min.y - 0.002f, bounds.center.z);
            var groundDiameter = Mathf.Max(20f, bounds.size.y * 20f);
            ground.transform.localScale = Vector3.one * Mathf.Max(0.5f, groundDiameter / 10f);
            var collider = ground.GetComponent<Collider>();
            if (collider != null)
            {
                Destroy(collider);
            }

            var shader = Shader.Find("Universal Render Pipeline/Lit");
            if (shader == null)
            {
                return;
            }

            _groundMaterial = new Material(shader)
            {
                name = "Runtime Preview Ground Material"
            };
            if (_groundMaterial.HasProperty("_BaseColor"))
            {
                _groundMaterial.SetColor("_BaseColor", new Color(0.16f, 0.18f, 0.22f, 1f));
            }
            if (_groundMaterial.HasProperty("_Metallic"))
            {
                _groundMaterial.SetFloat("_Metallic", 0f);
            }
            if (_groundMaterial.HasProperty("_Smoothness"))
            {
                _groundMaterial.SetFloat("_Smoothness", 0.28f);
            }
            ground.GetComponent<Renderer>().sharedMaterial = _groundMaterial;
        }

        private static bool TryCalculateBounds(GameObject root, out Bounds bounds)
        {
            var renderers = root != null
                ? root.GetComponentsInChildren<Renderer>(true)
                : Array.Empty<Renderer>();
            if (renderers.Length == 0)
            {
                bounds = default;
                return false;
            }

            bounds = renderers[0].bounds;
            for (var index = 1; index < renderers.Length; index++)
            {
                bounds.Encapsulate(renderers[index].bounds);
            }
            return true;
        }

private void EnsureEventSystem()
        {
            var eventSystem = FindFirstObjectByType<EventSystem>();
            if (eventSystem == null)
            {
                var eventSystemObject = new GameObject(
                    "Runtime Preview EventSystem",
                    typeof(EventSystem));
                eventSystemObject.transform.SetParent(transform, false);
                eventSystem = eventSystemObject.GetComponent<EventSystem>();
            }

            var inputSystemModuleType = Type.GetType(
                "UnityEngine.InputSystem.UI.InputSystemUIInputModule, Unity.InputSystem");
            if (inputSystemModuleType != null
                && typeof(BaseInputModule).IsAssignableFrom(inputSystemModuleType))
            {
                var inputModule = eventSystem.GetComponent(inputSystemModuleType) as BaseInputModule;
                if (inputModule == null)
                {
                    inputModule = eventSystem.gameObject.AddComponent(inputSystemModuleType) as BaseInputModule;
                }

                var assignDefaultActions = inputSystemModuleType.GetMethod(
                    "AssignDefaultActions",
                    System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.Public);
                assignDefaultActions?.Invoke(inputModule, null);

                foreach (var module in eventSystem.GetComponents<BaseInputModule>())
                {
                    module.enabled = module == inputModule;
                }
                return;
            }

            var standaloneModule = eventSystem.GetComponent<StandaloneInputModule>();
            if (standaloneModule == null)
            {
                standaloneModule = eventSystem.gameObject.AddComponent<StandaloneInputModule>();
            }
            standaloneModule.enabled = true;
        }

private void OnDestroy()
        {
            _lifetimeCancellation?.Cancel();
            _lifetimeCancellation?.Dispose();
            if (_animationPlayer != null)
            {
                _animationPlayer.AnimationChanged -= OnAnimationChanged;
            }
            if (_groundMaterial != null)
            {
                Destroy(_groundMaterial);
                _groundMaterial = null;
            }
            if (_runtimeSkyboxMaterial != null)
            {
                if (RenderSettings.skybox == _runtimeSkyboxMaterial)
                {
                    RenderSettings.skybox = _originalSkyboxMaterial;
                }
                Destroy(_runtimeSkyboxMaterial);
                _runtimeSkyboxMaterial = null;
            }
        }
    }
}
