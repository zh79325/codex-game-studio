using System;
using System.Collections.Generic;
using UnityEngine;

namespace ModelPreview.Runtime.Humanoid
{
    public enum HumanoidAnimationRole
    {
        Idle,
        Walk,
        Run,
        Punch,
        Kick,
        HeavyWeaponSwing
    }

    [DisallowMultipleComponent]
    public sealed class RuntimeHumanoidAnimationPlayer : MonoBehaviour
    {
        private const string DefaultControllerResource = "ModelPreview/HumanoidActionLibrary";

        private static readonly IReadOnlyDictionary<HumanoidAnimationRole, string> StateNames =
            new Dictionary<HumanoidAnimationRole, string>
            {
                { HumanoidAnimationRole.Idle, "Idle" },
                { HumanoidAnimationRole.Walk, "Walk" },
                { HumanoidAnimationRole.Run, "Run" },
                { HumanoidAnimationRole.Punch, "Punch" },
                { HumanoidAnimationRole.Kick, "Kick" },
                { HumanoidAnimationRole.HeavyWeaponSwing, "HeavyWeaponSwing" }
            };

        [SerializeField] private RuntimeAnimatorController actionController;
        [SerializeField] private string controllerResourcePath = DefaultControllerResource;
        [SerializeField, Min(0f)] private float transitionDuration = 0.12f;

        private readonly Dictionary<HumanoidAnimationRole, AnimationClip> _clips =
            new Dictionary<HumanoidAnimationRole, AnimationClip>();
        private readonly HashSet<HumanoidAnimationRole> _availableRoles =
            new HashSet<HumanoidAnimationRole>();

        private Animator _animator;
        private bool _configured;

        public event Action<HumanoidAnimationRole, AnimationClip> AnimationChanged;

        public bool IsConfigured => _configured
            && _animator != null
            && _animator.enabled
            && _animator.avatar != null
            && _animator.avatar.isValid
            && _animator.avatar.isHuman;
        public HumanoidAnimationRole CurrentRole { get; private set; }
        public string CurrentClipName => _clips.TryGetValue(CurrentRole, out var clip)
            ? clip.name
            : string.Empty;
        public IReadOnlyCollection<HumanoidAnimationRole> AvailableRoles => _availableRoles;

        public bool Configure(Animator animator)
        {
            ResetPlayer();
            _clips.Clear();
            _availableRoles.Clear();

            if (animator == null
                || animator.avatar == null
                || !animator.avatar.isValid
                || !animator.avatar.isHuman)
            {
                Debug.LogError("A valid Human Animator is required before animation playback.", this);
                return false;
            }

            var controller = actionController;
            if (controller == null && !string.IsNullOrWhiteSpace(controllerResourcePath))
            {
                controller = Resources.Load<RuntimeAnimatorController>(controllerResourcePath.Trim());
            }
            if (controller == null)
            {
                Debug.LogError(
                    "The shared Humanoid action controller is missing. Run "
                    + "Tools/Model Preview/Build Humanoid Action Library.",
                    this);
                return false;
            }

            // glTFast historically instantiated embedded glTF clips through the
            // Legacy Animation component. Disable any such component defensively;
            // mixing it with Animator causes two systems to overwrite the same
            // skeleton and produces severely twisted poses.
            foreach (var legacyAnimation in animator.GetComponentsInChildren<Animation>(true))
            {
                legacyAnimation.Stop();
                legacyAnimation.playAutomatically = false;
                legacyAnimation.enabled = false;
            }

            _animator = animator;
            _animator.runtimeAnimatorController = controller;
            _animator.applyRootMotion = false;
            _animator.cullingMode = AnimatorCullingMode.AlwaysAnimate;
            _animator.enabled = true;
            _animator.Rebind();
            _animator.Update(0f);

            var clipsByName = new Dictionary<string, AnimationClip>(StringComparer.OrdinalIgnoreCase);
            foreach (var clip in controller.animationClips)
            {
                if (clip != null)
                {
                    clipsByName[clip.name] = clip;
                }
            }

            foreach (var entry in StateNames)
            {
                var stateHash = Animator.StringToHash("Base Layer." + entry.Value);
                if (!_animator.HasState(0, stateHash))
                {
                    continue;
                }

                _availableRoles.Add(entry.Key);
                if (clipsByName.TryGetValue(entry.Value, out var clip))
                {
                    _clips[entry.Key] = clip;
                }
            }

            if (_availableRoles.Count == 0)
            {
                Debug.LogError("The shared controller contains no supported Humanoid states.", this);
                ResetPlayer();
                return false;
            }

            _configured = true;
            return Play(_availableRoles.Contains(HumanoidAnimationRole.Idle)
                ? HumanoidAnimationRole.Idle
                : FirstAvailableRole());
        }

        public bool IsAvailable(HumanoidAnimationRole role)
        {
            return IsConfigured && _availableRoles.Contains(role);
        }

        public bool Play(HumanoidAnimationRole role)
        {
            if (!IsConfigured || !_availableRoles.Contains(role))
            {
                return false;
            }

            var stateName = StateNames[role];
            if (transitionDuration > 0f)
            {
                _animator.CrossFadeInFixedTime(stateName, transitionDuration, 0, 0f);
            }
            else
            {
                _animator.Play(stateName, 0, 0f);
            }
            _animator.Update(0f);

            CurrentRole = role;
            _clips.TryGetValue(role, out var clip);
            AnimationChanged?.Invoke(role, clip);
            return true;
        }

        public bool PlayIdle() => Play(HumanoidAnimationRole.Idle);
        public bool PlayWalk() => Play(HumanoidAnimationRole.Walk);
        public bool PlayRun() => Play(HumanoidAnimationRole.Run);
        public bool PlayPunch() => Play(HumanoidAnimationRole.Punch);
        public bool PlayKick() => Play(HumanoidAnimationRole.Kick);
        public bool PlayHeavyWeaponSwing() => Play(HumanoidAnimationRole.HeavyWeaponSwing);

        private void Update()
        {
            if (!IsConfigured || _animator.IsInTransition(0))
            {
                return;
            }

            var state = _animator.GetCurrentAnimatorStateInfo(0);
            foreach (var entry in StateNames)
            {
                if (entry.Key == CurrentRole || !state.IsName(entry.Value))
                {
                    continue;
                }

                CurrentRole = entry.Key;
                _clips.TryGetValue(entry.Key, out var clip);
                AnimationChanged?.Invoke(entry.Key, clip);
                return;
            }
        }

        private HumanoidAnimationRole FirstAvailableRole()
        {
            foreach (HumanoidAnimationRole role in Enum.GetValues(typeof(HumanoidAnimationRole)))
            {
                if (_availableRoles.Contains(role))
                {
                    return role;
                }
            }
            return HumanoidAnimationRole.Idle;
        }

        private void ResetPlayer()
        {
            _configured = false;
            if (_animator != null)
            {
                _animator.enabled = true;
                _animator.runtimeAnimatorController = null;
            }
            _animator = null;
        }

        private void OnDestroy()
        {
            ResetPlayer();
        }
    }
}
