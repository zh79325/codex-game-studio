using System;
using System.Collections.Generic;
using UnityEngine;

namespace ModelPreview.Runtime.Humanoid
{
    public enum HumanoidAnimationRole
    {
        Idle,
        Walk,
        Run
    }

    [DisallowMultipleComponent]
    public sealed class RuntimeHumanoidAnimationPlayer : MonoBehaviour
    {
        private static readonly IReadOnlyDictionary<HumanoidAnimationRole, string[]> RolePatterns =
            new Dictionary<HumanoidAnimationRole, string[]>
            {
                { HumanoidAnimationRole.Idle, new[] { "idle", "standing_relax", "stand" } },
                { HumanoidAnimationRole.Walk, new[] { "walk" } },
                { HumanoidAnimationRole.Run, new[] { "run" } }
            };

        [SerializeField, Min(0f)] private float transitionDuration = 0.15f;

        private readonly Dictionary<HumanoidAnimationRole, AnimationClip> _clips =
            new Dictionary<HumanoidAnimationRole, AnimationClip>();

        private Animation _animation;
        private Animator _animator;
        private bool _configured;

        public event Action<HumanoidAnimationRole, AnimationClip> AnimationChanged;

        public bool IsConfigured => _configured && _animation != null;
        public HumanoidAnimationRole CurrentRole { get; private set; }
        public string CurrentClipName => IsConfigured && _clips.TryGetValue(CurrentRole, out var clip)
            ? clip.name
            : string.Empty;

        public bool Configure(Animator animator, IReadOnlyList<AnimationClip> sourceClips)
        {
            ResetPlayer();
            _clips.Clear();

            if (animator == null || animator.avatar == null || !animator.avatar.isValid || !animator.avatar.isHuman)
            {
                Debug.LogError("A valid Human Animator is required before animation playback.", this);
                return false;
            }

            if (sourceClips == null || sourceClips.Count == 0)
            {
                Debug.LogError("The loaded model contains no animation clips.", this);
                return false;
            }

            foreach (HumanoidAnimationRole role in Enum.GetValues(typeof(HumanoidAnimationRole)))
            {
                var clip = FindClip(sourceClips, RolePatterns[role]);
                if (clip != null)
                {
                    _clips[role] = clip;
                }
            }

            if (_clips.Count == 0)
            {
                Debug.LogError("No Idle, Walk, or Run clip could be identified.", this);
                return false;
            }

            _animator = animator;
            _animator.applyRootMotion = false;
            _animation = animator.GetComponent<Animation>();
            if (_animation == null)
            {
                _animation = animator.GetComponentInChildren<Animation>(true);
            }
            if (_animation == null)
            {
                _animation = animator.gameObject.AddComponent<Animation>();
            }

            _animation.enabled = true;
            foreach (var entry in _clips)
            {
                var clip = entry.Value;
                clip.legacy = true;
                if (_animation.GetClip(clip.name) == null)
                {
                    _animation.AddClip(clip, clip.name);
                }

                var state = _animation[clip.name];
                if (state != null)
                {
                    state.wrapMode = WrapMode.Loop;
                    state.speed = 1f;
                    state.layer = 0;
                }
            }

            _animator.enabled = false;
            _configured = true;

            var initialRole = _clips.ContainsKey(HumanoidAnimationRole.Idle)
                ? HumanoidAnimationRole.Idle
                : FirstAvailableRole();
            return Play(initialRole);
        }

        public bool IsAvailable(HumanoidAnimationRole role)
        {
            return IsConfigured && _clips.ContainsKey(role);
        }

        public bool Play(HumanoidAnimationRole role)
        {
            if (!IsConfigured || !_clips.TryGetValue(role, out var clip))
            {
                return false;
            }

            var state = _animation[clip.name];
            if (state == null)
            {
                return false;
            }

            state.wrapMode = WrapMode.Loop;
            state.time = 0f;
            if (transitionDuration > 0f && _animation.isPlaying)
            {
                _animation.CrossFade(clip.name, transitionDuration, PlayMode.StopAll);
            }
            else
            {
                _animation.Play(clip.name, PlayMode.StopAll);
            }

            CurrentRole = role;
            AnimationChanged?.Invoke(role, clip);
            return true;
        }

        public bool PlayIdle()
        {
            return Play(HumanoidAnimationRole.Idle);
        }

        public bool PlayWalk()
        {
            return Play(HumanoidAnimationRole.Walk);
        }

        public bool PlayRun()
        {
            return Play(HumanoidAnimationRole.Run);
        }

        private static AnimationClip FindClip(
            IReadOnlyList<AnimationClip> clips,
            IReadOnlyList<string> patterns)
        {
            foreach (var pattern in patterns)
            {
                for (var index = 0; index < clips.Count; index++)
                {
                    var clip = clips[index];
                    if (clip != null
                        && clip.name.IndexOf(pattern, StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        return clip;
                    }
                }
            }

            return null;
        }

        private HumanoidAnimationRole FirstAvailableRole()
        {
            foreach (HumanoidAnimationRole role in Enum.GetValues(typeof(HumanoidAnimationRole)))
            {
                if (_clips.ContainsKey(role))
                {
                    return role;
                }
            }

            return HumanoidAnimationRole.Idle;
        }

        private void ResetPlayer()
        {
            _configured = false;
            if (_animation != null)
            {
                _animation.Stop();
            }
            if (_animator != null)
            {
                _animator.enabled = true;
            }

            _animation = null;
            _animator = null;
        }

        private void OnDestroy()
        {
            ResetPlayer();
        }
    }
}

