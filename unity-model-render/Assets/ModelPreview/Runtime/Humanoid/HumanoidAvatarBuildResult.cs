using System.Collections.Generic;
using UnityEngine;

namespace ModelPreview.Runtime.Humanoid
{
    public enum HumanoidAvatarBuildErrorCode
    {
        None,
        InvalidModel,
        DuplicateTransformNames,
        MissingRequiredBones,
        InvalidHierarchy,
        AvatarBuildFailed,
        BoneVerificationFailed
    }

    public sealed class HumanoidAvatarBuildResult
    {
        private HumanoidAvatarBuildResult(
            Avatar avatar,
            Animator animator,
            IReadOnlyDictionary<HumanBodyBones, Transform> boneMap,
            HumanoidAvatarBuildErrorCode errorCode,
            string message,
            IReadOnlyList<string> diagnostics)
        {
            Avatar = avatar;
            Animator = animator;
            BoneMap = boneMap;
            ErrorCode = errorCode;
            Message = message ?? string.Empty;
            Diagnostics = diagnostics;
        }

        public bool Succeeded => ErrorCode == HumanoidAvatarBuildErrorCode.None
            && Avatar != null
            && Avatar.isValid
            && Avatar.isHuman;

        public Avatar Avatar { get; }
        public Animator Animator { get; }
        public IReadOnlyDictionary<HumanBodyBones, Transform> BoneMap { get; }
        public HumanoidAvatarBuildErrorCode ErrorCode { get; }
        public string Message { get; }
        public IReadOnlyList<string> Diagnostics { get; }

        internal static HumanoidAvatarBuildResult Success(
            Avatar avatar,
            Animator animator,
            IReadOnlyDictionary<HumanBodyBones, Transform> boneMap,
            IReadOnlyList<string> diagnostics)
        {
            return new HumanoidAvatarBuildResult(
                avatar,
                animator,
                boneMap,
                HumanoidAvatarBuildErrorCode.None,
                string.Empty,
                diagnostics);
        }

        internal static HumanoidAvatarBuildResult Failure(
            HumanoidAvatarBuildErrorCode errorCode,
            string message,
            IReadOnlyList<string> diagnostics = null)
        {
            return new HumanoidAvatarBuildResult(
                null,
                null,
                null,
                errorCode,
                message,
                diagnostics ?? new string[0]);
        }
    }
}
