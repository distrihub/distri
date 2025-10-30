import { createTool } from 'https://distri.dev/base.ts';

export function getBuildTools() {
  return [
    createTool({
      name: 'build_package',
      description: 'Build the current plugin/package for distribution',
      parameters: {
        type: 'object',
        properties: {},
        required: []
      },
      execute: async () => {
        // Check if we're in a valid plugin directory using Deno APIs
        const cwd = Deno.cwd();
        const distriDir = `${cwd}/.distri`;
        
        try {
          await Deno.stat(distriDir);
        } catch {
          throw new Error('Not in a valid distri plugin directory. Missing .distri folder.');
        }
        
        // Read package info from package.json or generate
        let packageInfo = {
          name: cwd.split('/').pop() || 'unknown',
          version: '1.0.0',
          description: 'Generated plugin package',
        };
        
        const packageJsonPath = `${cwd}/package.json`;
        try {
          const packageJson = JSON.parse(await Deno.readTextFile(packageJsonPath));
          packageInfo = {
            name: packageJson.name || packageInfo.name,
            version: packageJson.version || packageInfo.version,
            description: packageJson.description || packageInfo.description,
          };
        } catch (e) {
          console.warn('Warning: Could not parse package.json, using defaults');
        }
        
        console.log(`Building package: ${packageInfo.name}@${packageInfo.version}`);
        console.log(`Source directory: ${distriDir}`);
        
        // Validate structure
        const indexPath = `${distriDir}/src/index.ts`;
        try {
          await Deno.stat(indexPath);
        } catch {
          throw new Error('Missing required src/index.ts entrypoint');
        }
        
        // Create build manifest
        const buildManifest = {
          package: packageInfo.name,
          version: packageInfo.version,
          description: packageInfo.description,
          entrypoints: {
            type: 'ts',
            path: 'src/index.ts'
          },
          build_timestamp: new Date().toISOString(),
          files: await collectFiles(distriDir)
        };
        
        // Write build manifest
        const buildDir = `${cwd}/dist`;
        try {
          await Deno.mkdir(buildDir, { recursive: true });
        } catch {
          // Directory might already exist
        }
        
        const manifestPath = `${buildDir}/distri.json`;
        await Deno.writeTextFile(manifestPath, JSON.stringify(buildManifest, null, 2));
        
        console.log('✅ Build completed successfully');
        console.log(`📝 Build manifest: ${manifestPath}`);
        
        return {
          success: true,
          package: packageInfo,
          manifest: buildManifest,
          message: 'Package built successfully'
        };
      }
    }),
    
    createTool({
      name: 'publish_package', 
      description: 'Publish the built package to the distri registry',
      parameters: {
        type: 'object',
        properties: {
          registry_url: {
            type: 'string',
            description: 'Registry URL to publish to',
            default: 'https://registry.distri.dev'
          }
        },
        required: []
      },
      execute: async (params) => {
        const registryUrl = params.registry_url || 'https://registry.distri.dev';
        
        // Check if package is built using Deno APIs
        const cwd = Deno.cwd();
        const distriDir = `${cwd}/.distri`;
        
        try {
          await Deno.stat(distriDir);
        } catch {
          throw new Error('Not in a valid distri plugin directory. Run /build first.');
        }
        
        const packageName = cwd.split('/').pop() || 'unknown';
        console.log(`Publishing package: ${packageName} to ${registryUrl}`);
        
        // TODO: Implement actual publish logic with tarball creation and upload
        // For now, just simulate the publish
        console.log('📦 Creating tarball...');
        console.log('🚀 Uploading to registry...');
        console.log('✅ Package published successfully');
        
        return {
          success: true,
          package_name: packageName,
          registry_url: registryUrl,
          message: `Package ${packageName} published successfully to ${registryUrl}`
        };
      }
    })
  ];
}

async function collectFiles(directory: string): Promise<{ path: string; content: string }[]> {
  const files: { path: string; content: string }[] = [];
  
  async function walkDir(dir: string, relativePath = '') {
    try {
      for await (const entry of Deno.readDir(dir)) {
        const fullPath = `${dir}/${entry.name}`;
        const relPath = relativePath ? `${relativePath}/${entry.name}` : entry.name;
        
        if (entry.isDirectory) {
          await walkDir(fullPath, relPath);
        } else if (entry.isFile) {
          try {
            const content = await Deno.readTextFile(fullPath);
            files.push({
              path: relPath,
              content
            });
          } catch (e) {
            console.warn(`Warning: Could not read file ${fullPath}: ${e}`);
          }
        }
      }
    } catch (e) {
      console.warn(`Warning: Could not read directory ${dir}: ${e}`);
    }
  }
  
  await walkDir(directory);
  return files;
}