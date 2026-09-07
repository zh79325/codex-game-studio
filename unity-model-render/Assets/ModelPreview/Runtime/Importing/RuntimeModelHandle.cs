using System;
using System.Collections.Generic;
using System.Linq;
using GLTFast;
using UnityEngine;

namespace ModelPreview.Runtime.Importing
{
    public sealed class RuntimeModelHandle : IDisposable
    {
        private GltfImport _import;
        private GameObject _root;
        private readonly AnimationClip[] _animationClips;

        internal RuntimeModelHandle(
            string sourcePath,
            GltfImport import,
            GameObject root,
            IReadOnlyList<AnimationClip> animationClips)
        {
            SourcePath = sourcePath;
            _import = import ?? throw new ArgumentNullException(nameof(import));
            _root = root ?? throw new ArgumentNullException(nameof(root));
            _animationClips = animationClips == null
                ? Array.Empty<AnimationClip>()
                : animationClips.Where(clip => clip != null).ToArray();
        }

        public string SourcePath { get; }
        public GameObject Root => _root;
        public IReadOnlyList<AnimationClip> AnimationClips => _animationClips;
        public bool IsDisposed { get; private set; }

        public void Dispose()
        {
            if (IsDisposed)
            {
                return;
            }

            IsDisposed = true;

            if (_root != null)
            {
                _root.SetActive(false);
                if (Application.isPlaying)
                {
                    UnityEngine.Object.Destroy(_root);
                }
                else
                {
                    UnityEngine.Object.DestroyImmediate(_root);
                }

                _root = null;
            }

            _import?.Dispose();
            _import = null;
        }
    }
}
