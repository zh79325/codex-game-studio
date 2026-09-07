using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using UnityEngine;

namespace ModelPreview.Runtime.Humanoid
{
    internal static class HumanoidBoneResolver
    {
        private static readonly Dictionary<HumanBodyBones, string[]> Aliases =
            new Dictionary<HumanBodyBones, string[]>
            {
                { HumanBodyBones.Hips, new[] { "hip", "hips", "pelvis" } },
                { HumanBodyBones.Spine, new[] { "waist", "spine", "spine01", "spine1" } },
                { HumanBodyBones.Chest, new[] { "chest", "spine01", "spine1", "spine02", "spine2" } },
                { HumanBodyBones.UpperChest, new[] { "upperchest", "spine02", "spine2", "chest2" } },
                { HumanBodyBones.Neck, new[] { "neck", "neck01", "neck1", "necktwist02", "necktwist01" } },
                { HumanBodyBones.Head, new[] { "head" } },
                { HumanBodyBones.LeftShoulder, new[] { "leftshoulder", "lshoulder", "lclavicle", "leftclavicle" } },
                { HumanBodyBones.RightShoulder, new[] { "rightshoulder", "rshoulder", "rclavicle", "rightclavicle" } },
                { HumanBodyBones.LeftUpperArm, new[] { "leftupperarm", "lupperarm", "leftarm", "larm" } },
                { HumanBodyBones.RightUpperArm, new[] { "rightupperarm", "rupperarm", "rightarm", "rarm" } },
                { HumanBodyBones.LeftLowerArm, new[] { "leftlowerarm", "llowerarm", "leftforearm", "lforearm" } },
                { HumanBodyBones.RightLowerArm, new[] { "rightlowerarm", "rlowerarm", "rightforearm", "rforearm" } },
                { HumanBodyBones.LeftHand, new[] { "lefthand", "lhand" } },
                { HumanBodyBones.RightHand, new[] { "righthand", "rhand" } },
                { HumanBodyBones.LeftUpperLeg, new[] { "leftupperleg", "lupperleg", "leftthigh", "lthigh" } },
                { HumanBodyBones.RightUpperLeg, new[] { "rightupperleg", "rupperleg", "rightthigh", "rthigh" } },
                { HumanBodyBones.LeftLowerLeg, new[] { "leftlowerleg", "llowerleg", "leftcalf", "lcalf", "leftshin", "lshin" } },
                { HumanBodyBones.RightLowerLeg, new[] { "rightlowerleg", "rlowerleg", "rightcalf", "rcalf", "rightshin", "rshin" } },
                { HumanBodyBones.LeftFoot, new[] { "leftfoot", "lfoot", "leftankle", "lankle" } },
                { HumanBodyBones.RightFoot, new[] { "rightfoot", "rfoot", "rightankle", "rankle" } },
                { HumanBodyBones.LeftToes, new[] { "lefttoes", "lefttoebase", "ltoebase", "ltoe" } },
                { HumanBodyBones.RightToes, new[] { "righttoes", "righttoebase", "rtoebase", "rtoe" } }
            };

        internal static bool TryResolve(
            Transform modelRoot,
            HumanoidAvatarProfile profile,
            out Dictionary<HumanBodyBones, Transform> boneMap,
            out HumanoidAvatarBuildErrorCode errorCode,
            out string errorMessage,
            out List<string> diagnostics)
        {
            boneMap = new Dictionary<HumanBodyBones, Transform>();
            diagnostics = new List<string>();
            errorCode = HumanoidAvatarBuildErrorCode.None;
            errorMessage = string.Empty;

            if (modelRoot == null)
            {
                errorCode = HumanoidAvatarBuildErrorCode.InvalidModel;
                errorMessage = "The model root is null.";
                return false;
            }

            var transforms = modelRoot.GetComponentsInChildren<Transform>(true);
            var duplicateNames = transforms
                .GroupBy(item => item.name, StringComparer.OrdinalIgnoreCase)
                .Where(group => group.Count() > 1)
                .Select(group => group.Key)
                .OrderBy(name => name)
                .ToArray();
            if (duplicateNames.Length > 0)
            {
                errorCode = HumanoidAvatarBuildErrorCode.DuplicateTransformNames;
                errorMessage = "Humanoid Avatar requires unique transform names. Duplicates: "
                    + string.Join(", ", duplicateNames);
                return false;
            }

            foreach (var entry in Aliases)
            {
                var transform = ResolveTransform(modelRoot, transforms, profile, entry.Key, entry.Value, out var source);
                if (transform == null)
                {
                    continue;
                }

                boneMap[entry.Key] = transform;
                diagnostics.Add(entry.Key + " -> " + transform.name + " (" + source + ")");
            }

            var missingRequired = new List<string>();
            for (var index = 0; index < HumanTrait.BoneCount; index++)
            {
                if (!HumanTrait.RequiredBone(index))
                {
                    continue;
                }

                var humanBone = (HumanBodyBones)index;
                if (!boneMap.ContainsKey(humanBone))
                {
                    missingRequired.Add(HumanTrait.BoneName[index]);
                }
            }

            if (missingRequired.Count > 0)
            {
                errorCode = HumanoidAvatarBuildErrorCode.MissingRequiredBones;
                errorMessage = "Missing required Humanoid bones: " + string.Join(", ", missingRequired);
                return false;
            }

            var duplicateMappings = boneMap
                .GroupBy(pair => pair.Value)
                .Where(group => group.Count() > 1)
                .Select(group => group.Key.name)
                .ToArray();
            if (duplicateMappings.Length > 0)
            {
                errorCode = HumanoidAvatarBuildErrorCode.InvalidHierarchy;
                errorMessage = "One transform was mapped to multiple Humanoid bones: "
                    + string.Join(", ", duplicateMappings);
                return false;
            }

            if (!ValidateHierarchy(boneMap, out errorMessage))
            {
                errorCode = HumanoidAvatarBuildErrorCode.InvalidHierarchy;
                return false;
            }

            return true;
        }

        private static Transform ResolveTransform(
            Transform modelRoot,
            Transform[] transforms,
            HumanoidAvatarProfile profile,
            HumanBodyBones humanBone,
            string[] aliases,
            out string source)
        {
            if (profile != null && profile.TryGetTransformPath(humanBone, out var path))
            {
                var profileTransform = FindByPathOrName(modelRoot, transforms, path);
                source = "profile";
                return profileTransform;
            }

            foreach (var alias in aliases)
            {
                var normalizedAlias = NormalizeName(alias);
                var exact = transforms.FirstOrDefault(
                    candidate => NormalizeName(candidate.name) == normalizedAlias);
                if (exact != null)
                {
                    source = "alias";
                    return exact;
                }
            }

            foreach (var alias in aliases)
            {
                var normalizedAlias = NormalizeName(alias);
                var suffixMatches = transforms
                    .Where(candidate => NormalizeName(candidate.name).EndsWith(normalizedAlias, StringComparison.Ordinal))
                    .ToArray();
                if (suffixMatches.Length == 1)
                {
                    source = "alias suffix";
                    return suffixMatches[0];
                }
            }

            source = string.Empty;
            return null;
        }

        private static Transform FindByPathOrName(
            Transform modelRoot,
            Transform[] transforms,
            string pathOrName)
        {
            var normalizedPath = pathOrName.Trim().Trim('/');
            if (normalizedPath.StartsWith(modelRoot.name + "/", StringComparison.Ordinal))
            {
                normalizedPath = normalizedPath.Substring(modelRoot.name.Length + 1);
            }

            var byPath = modelRoot.Find(normalizedPath);
            if (byPath != null)
            {
                return byPath;
            }

            return transforms.FirstOrDefault(
                candidate => string.Equals(candidate.name, pathOrName, StringComparison.OrdinalIgnoreCase));
        }

        private static string NormalizeName(string value)
        {
            var builder = new StringBuilder(value.Length);
            foreach (var character in value)
            {
                if (char.IsLetterOrDigit(character))
                {
                    builder.Append(char.ToLowerInvariant(character));
                }
            }

            return builder.ToString();
        }

        private static bool ValidateHierarchy(
            IReadOnlyDictionary<HumanBodyBones, Transform> map,
            out string error)
        {
            if (!IsAncestor(map[HumanBodyBones.Hips], map[HumanBodyBones.Spine])
                || !IsAncestor(map[HumanBodyBones.Hips], map[HumanBodyBones.LeftUpperLeg])
                || !IsAncestor(map[HumanBodyBones.Hips], map[HumanBodyBones.RightUpperLeg]))
            {
                error = "Hips must be the common ancestor of the spine and both upper legs.";
                return false;
            }

            if (!ValidateChain(map, HumanBodyBones.LeftUpperLeg, HumanBodyBones.LeftLowerLeg, HumanBodyBones.LeftFoot)
                || !ValidateChain(map, HumanBodyBones.RightUpperLeg, HumanBodyBones.RightLowerLeg, HumanBodyBones.RightFoot)
                || !ValidateChain(map, HumanBodyBones.LeftUpperArm, HumanBodyBones.LeftLowerArm, HumanBodyBones.LeftHand)
                || !ValidateChain(map, HumanBodyBones.RightUpperArm, HumanBodyBones.RightLowerArm, HumanBodyBones.RightHand))
            {
                error = "One or more required arm or leg bone chains are not hierarchical.";
                return false;
            }

            if (!IsAncestor(map[HumanBodyBones.Spine], map[HumanBodyBones.Head]))
            {
                error = "Head must be a descendant of Spine.";
                return false;
            }

            error = string.Empty;
            return true;
        }

        private static bool ValidateChain(
            IReadOnlyDictionary<HumanBodyBones, Transform> map,
            HumanBodyBones root,
            HumanBodyBones middle,
            HumanBodyBones tip)
        {
            return IsAncestor(map[root], map[middle]) && IsAncestor(map[middle], map[tip]);
        }

        private static bool IsAncestor(Transform ancestor, Transform descendant)
        {
            return ancestor != null
                && descendant != null
                && descendant != ancestor
                && descendant.IsChildOf(ancestor);
        }
    }
}
