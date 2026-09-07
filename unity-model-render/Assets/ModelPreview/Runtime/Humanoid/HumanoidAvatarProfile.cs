using System;
using System.Collections.Generic;
using UnityEngine;

namespace ModelPreview.Runtime.Humanoid
{
    [Serializable]
    public struct HumanoidBonePathOverride
    {
        public HumanBodyBones humanBone;
        public string transformPath;
    }

    [CreateAssetMenu(
        fileName = "HumanoidAvatarProfile",
        menuName = "Model Preview/Humanoid Avatar Profile")]
    public sealed class HumanoidAvatarProfile : ScriptableObject
    {
        [SerializeField] private List<HumanoidBonePathOverride> boneOverrides = new List<HumanoidBonePathOverride>();

        public bool TryGetTransformPath(HumanBodyBones humanBone, out string transformPath)
        {
            foreach (var entry in boneOverrides)
            {
                if (entry.humanBone == humanBone && !string.IsNullOrWhiteSpace(entry.transformPath))
                {
                    transformPath = entry.transformPath.Trim();
                    return true;
                }
            }

            transformPath = string.Empty;
            return false;
        }
    }
}
