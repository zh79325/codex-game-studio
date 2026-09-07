using UnityEngine;

namespace ModelPreview.Runtime.Humanoid
{
    public sealed class RuntimeHumanoidAvatarOwner : MonoBehaviour
    {
        public Avatar Avatar { get; private set; }
        public Animator Animator { get; private set; }

        internal void Assign(Animator animator, Avatar avatar)
        {
            Release();
            Animator = animator;
            Avatar = avatar;
        }

        internal void Release()
        {
            if (Animator != null && Animator.avatar == Avatar)
            {
                Animator.avatar = null;
            }

            if (Avatar != null)
            {
                if (Application.isPlaying)
                {
                    Destroy(Avatar);
                }
                else
                {
                    DestroyImmediate(Avatar);
                }
            }

            Avatar = null;
            Animator = null;
        }

        private void OnDestroy()
        {
            Release();
        }
    }
}
