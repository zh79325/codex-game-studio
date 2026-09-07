using System;
using System.Collections.Generic;
using ModelPreview.Runtime.Importing;
using UnityEngine;

namespace ModelPreview.Runtime.Humanoid
{
    public static class RuntimeHumanoidAvatarBuilder
    {
        public static HumanoidAvatarBuildResult Build(
            RuntimeModelHandle modelHandle,
            HumanoidAvatarProfile profile = null)
        {
            if (modelHandle == null || modelHandle.IsDisposed || modelHandle.Root == null)
            {
                return HumanoidAvatarBuildResult.Failure(
                    HumanoidAvatarBuildErrorCode.InvalidModel,
                    "The runtime model handle is null, disposed, or has no model root.");
            }

            var modelRoot = modelHandle.Root;
            if (!HumanoidBoneResolver.TryResolve(
                    modelRoot.transform,
                    profile,
                    out var boneMap,
                    out var errorCode,
                    out var errorMessage,
                    out var diagnostics))
            {
                return HumanoidAvatarBuildResult.Failure(
                    errorCode,
                    errorMessage,
                    diagnostics);
            }

            Avatar avatar = null;
            RuntimeHumanoidAvatarOwner owner = null;

            try
            {
                var description = CreateHumanDescription(modelRoot.transform, boneMap);
                avatar = AvatarBuilder.BuildHumanAvatar(modelRoot, description);
                if (avatar == null || !avatar.isValid || !avatar.isHuman)
                {
                    DestroyAvatar(avatar);
                    return HumanoidAvatarBuildResult.Failure(
                        HumanoidAvatarBuildErrorCode.AvatarBuildFailed,
                        "Unity failed to build a valid Human Avatar from the resolved skeleton.",
                        diagnostics);
                }

                avatar.name = modelRoot.name + " Runtime Humanoid Avatar";

                var animator = modelRoot.GetComponent<Animator>();
                if (animator == null)
                {
                    animator = modelRoot.AddComponent<Animator>();
                }

                owner = modelRoot.GetComponent<RuntimeHumanoidAvatarOwner>();
                if (owner == null)
                {
                    owner = modelRoot.AddComponent<RuntimeHumanoidAvatarOwner>();
                }

                owner.Assign(animator, avatar);
                animator.avatar = avatar;
                animator.applyRootMotion = false;
                animator.Rebind();
                animator.Update(0f);

                if (!TryVerifyBoneMap(animator, boneMap, diagnostics, out errorMessage))
                {
                    owner.Release();
                    avatar = null;
                    return HumanoidAvatarBuildResult.Failure(
                        HumanoidAvatarBuildErrorCode.BoneVerificationFailed,
                        errorMessage,
                        diagnostics);
                }

                diagnostics.Add("Avatar is valid and human.");
                diagnostics.Add("Animator.applyRootMotion = false.");
                return HumanoidAvatarBuildResult.Success(
                    avatar,
                    animator,
                    new Dictionary<HumanBodyBones, Transform>(boneMap),
                    diagnostics.ToArray());
            }
            catch (Exception exception)
            {
                if (owner != null && owner.Avatar == avatar)
                {
                    owner.Release();
                    avatar = null;
                }

                DestroyAvatar(avatar);
                diagnostics.Add(exception.GetType().Name + ": " + exception.Message);
                return HumanoidAvatarBuildResult.Failure(
                    HumanoidAvatarBuildErrorCode.AvatarBuildFailed,
                    "Humanoid Avatar construction failed: " + exception.Message,
                    diagnostics);
            }
        }

        private static HumanDescription CreateHumanDescription(
            Transform modelRoot,
            IReadOnlyDictionary<HumanBodyBones, Transform> boneMap)
        {
            var humanBones = new HumanBone[boneMap.Count];
            var humanIndex = 0;
            foreach (var entry in boneMap)
            {
                humanBones[humanIndex++] = new HumanBone
                {
                    humanName = HumanTrait.BoneName[(int)entry.Key],
                    boneName = entry.Value.name,
                    limit = new HumanLimit { useDefaultValues = true }
                };
            }

            var transforms = modelRoot.GetComponentsInChildren<Transform>(true);
            var skeletonBones = new SkeletonBone[transforms.Length];
            for (var index = 0; index < transforms.Length; index++)
            {
                var transform = transforms[index];
                skeletonBones[index] = new SkeletonBone
                {
                    name = transform.name,
                    position = transform.localPosition,
                    rotation = transform.localRotation,
                    scale = transform.localScale
                };
            }

            return new HumanDescription
            {
                human = humanBones,
                skeleton = skeletonBones,
                upperArmTwist = 0.5f,
                lowerArmTwist = 0.5f,
                upperLegTwist = 0.5f,
                lowerLegTwist = 0.5f,
                armStretch = 0.05f,
                legStretch = 0.05f,
                feetSpacing = 0f,
                hasTranslationDoF = false
            };
        }

        private static bool TryVerifyBoneMap(
            Animator animator,
            IReadOnlyDictionary<HumanBodyBones, Transform> expectedBoneMap,
            ICollection<string> diagnostics,
            out string errorMessage)
        {
            foreach (var entry in expectedBoneMap)
            {
                var actual = animator.GetBoneTransform(entry.Key);
                if (actual == null)
                {
                    errorMessage = "Animator did not expose mapped bone " + entry.Key + ".";
                    return false;
                }

                if (actual != entry.Value)
                {
                    errorMessage = "Animator bone mismatch for " + entry.Key
                        + ": expected " + entry.Value.name
                        + ", got " + actual.name + ".";
                    return false;
                }

                diagnostics.Add("Verified " + entry.Key + " -> " + actual.name + ".");
            }

            errorMessage = string.Empty;
            return true;
        }

        private static void DestroyAvatar(Avatar avatar)
        {
            if (avatar == null)
            {
                return;
            }

            if (Application.isPlaying)
            {
                UnityEngine.Object.Destroy(avatar);
            }
            else
            {
                UnityEngine.Object.DestroyImmediate(avatar);
            }
        }
    }
}
